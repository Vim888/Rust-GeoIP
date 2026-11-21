use serde::Serialize;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    net::IpAddr,
    os::unix::ffi::OsStrExt,
    path::Path,
};
#[repr(C)]
struct Raw {
    country_code: *const c_char,
    country_name: *const c_char,
    cc: *const c_char,
    cc3: *const c_char,
    cn: *const c_char,
    region: *const c_char,
    city: *const c_char,
    postal: *const c_char,
    region_name: *const c_char,
    tz: *const c_char,
    asn: *const c_char,
    org: *const c_char,
    isp: *const c_char,
    latitude: f64,
    longitude: f64,
    area_code: i32,
    metro_code: i32,
    record: *mut c_void,
}
extern "C" {
    fn open_geo(path: *const c_char, edition: i32) -> *mut c_void;
    fn GeoIP_delete(db: *mut c_void);
    fn read_geo(dbs: *const *mut c_void, v: i32, ip: *const u8, out: *mut Raw);
    fn drop_geo(out: *mut Raw);
}
impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            drop_geo(self);
        }
    }
}
impl Raw {
    fn text(&self, p: *const c_char) -> Option<&str> {
        if p.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(p) }.to_str().ok()
        }
    }
}
#[derive(Serialize)]
struct City<'a> {
    country_code: Option<&'a str>,
    country_code3: Option<&'a str>,
    country_name: Option<&'a str>,
    region: Option<&'a str>,
    city: Option<&'a str>,
    postal_code: Option<&'a str>,
    region_name: Option<&'a str>,
    time_zone: Option<&'a str>,
    // libGeoIP stores coordinates as float: f32 prints 37.751, not 37.750999450683594.
    latitude: f32,
    longitude: f32,
    area_code: i32,
    metro_code: i32,
    dma_code: i32,
}
#[derive(Serialize)]
struct Reply<'a> {
    country_code: Option<&'a str>,
    country_name: Option<&'a str>,
    city: Option<City<'a>>,
    asn: Option<&'a str>,
    organization: Option<&'a str>,
    isp: Option<&'a str>,
}
impl Databases {
    pub fn answer(&self, ip: IpAddr, out: &mut Vec<u8>) {
        let mut r: Raw = unsafe { std::mem::zeroed() };
        unsafe {
            match ip {
                IpAddr::V4(v) => {
                    read_geo(self.handles[..5].as_ptr(), 0, v.octets().as_ptr(), &mut r)
                }
                IpAddr::V6(v) => {
                    read_geo(self.handles[5..].as_ptr(), 1, v.octets().as_ptr(), &mut r)
                }
            }
        }
        let city = (!r.record.is_null()).then(|| City {
            country_code: r.text(r.cc),
            country_code3: r.text(r.cc3),
            country_name: r.text(r.cn),
            region: r.text(r.region),
            city: r.text(r.city),
            postal_code: r.text(r.postal),
            region_name: r.text(r.region_name),
            time_zone: r.text(r.tz),
            latitude: r.latitude as f32,
            longitude: r.longitude as f32,
            area_code: r.area_code,
            metro_code: r.metro_code,
            dma_code: r.metro_code,
        });
        let reply = Reply {
            country_code: r.text(r.country_code),
            country_name: r.text(r.country_name),
            city,
            asn: r.text(r.asn),
            organization: r.text(r.org),
            isp: r.text(r.isp),
        };
        serde_json::to_writer(out, &reply).unwrap();
    }
}
pub struct Databases {
    handles: Vec<*mut c_void>,
    pub oldest: chrono::NaiveDate,
}

// Ownership moves from the loader to the single-threaded request loop.
// No libGeoIP handle is ever used by two threads at once.
unsafe impl Send for Databases {}

impl Drop for Databases {
    fn drop(&mut self) {
        for db in self.handles.drain(..) {
            unsafe { GeoIP_delete(db) };
        }
    }
}

impl Databases {
    pub fn load(directory: &Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let lock = File::open(directory)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let active = directory.join("active");
        let root = match active.symlink_metadata() {
            Ok(_) => active.canonicalize()?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => directory.to_owned(),
            Err(e) => return Err(e),
        };
        let mut set = Self {
            handles: Vec::with_capacity(10),
            oldest: chrono::NaiveDate::MAX,
        };
        for family in 0..2 {
            let suffix = ["", "v6"][family];
            for (base, editions, width) in [
                ("GeoIP", [1, 12], 6),
                ("GeoIPCity", [2, 30], 6),
                ("GeoIPASNum", [9, 21], 6),
                ("GeoIPOrg", [5, 23], 8),
                ("GeoIPISP", [4, 22], 8),
            ] {
                let edition = editions[family];
                let path = root.join(format!("{base}{suffix}.dat"));
                let invalid = || {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Invalid GeoIP database {}", path.display()),
                    )
                };
                let mut file = File::open(&path)?;
                let size = file.metadata()?.len();
                // mailfud Legacy footer: YYYYMMDD, three FF bytes, edition, LE24 tree size.
                let mut footer = [0; 15];
                file.seek(SeekFrom::End(-15))?;
                file.read_exact(&mut footer)?;
                let nodes = u32::from_le_bytes([footer[12], footer[13], footer[14], 0]);
                if footer[8..12] != [255, 255, 255, edition]
                    || nodes == 0
                    || u64::from(nodes) * width > size - 15
                {
                    return Err(invalid());
                }
                let date = std::str::from_utf8(&footer[..8])
                    .ok()
                    .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y%m%d").ok())
                    .ok_or_else(invalid)?;
                let name = CString::new(path.as_os_str().as_bytes())?;
                let handle = unsafe { open_geo(name.as_ptr(), edition.into()) };
                if handle.is_null() {
                    return Err(invalid());
                }
                set.handles.push(handle);
                set.oldest = set.oldest.min(date);
            }
        }
        Ok(set)
    }
}
