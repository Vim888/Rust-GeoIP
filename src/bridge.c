#include <GeoIP.h>
#include <GeoIPCity.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
  const char *country_code, *country_name, *cc, *cc3, *cn, *region, *city,
      *postal, *region_name, *tz, *asn, *org, *isp;
  double latitude, longitude;
  int area_code, metro_code;
  GeoIPRecord *record;
} Reply;
GeoIP *open_geo(const char *path, int edition) {
  GeoIP *db = GeoIP_open(path, GEOIP_MEMORY_CACHE);
  if (!db)
    return NULL;
  if (GeoIP_database_edition(db) != edition || !db->cache) {
    GeoIP_delete(db);
    return NULL;
  }
  GeoIP_set_charset(db, GEOIP_CHARSET_UTF8);
  return db;
}
void read_geo(GeoIP *const *d, int v, const unsigned char *raw, Reply *out) {
  uint32_t n = 0;
  geoipv6_t a = {0};
  if (v)
    memcpy(&a, raw, 16);
  else {
    memcpy(&n, raw, 4);
    n = ntohl(n);
  }
  int id = v ? GeoIP_id_by_ipnum_v6(d[0], a) : GeoIP_id_by_ipnum(d[0], n);
  if (id) {
    out->country_code = GeoIP_code_by_id(id);
    out->country_name = GeoIP_country_name_by_id(d[0], id);
  }
  GeoIPRecord *r =
      v ? GeoIP_record_by_ipnum_v6(d[1], a) : GeoIP_record_by_ipnum(d[1], n);
  if (r) {
    out->record = r;
    out->cc = r->country_code;
    out->cc3 = r->country_code3;
    out->cn = r->country_name;
    out->region = r->region;
    out->city = r->city;
    out->postal = r->postal_code;
    out->region_name = GeoIP_region_name_by_code(r->country_code, r->region);
    out->tz = GeoIP_time_zone_by_country_and_region(r->country_code, r->region);
    out->latitude = r->latitude;
    out->longitude = r->longitude;
    out->area_code = r->area_code;
    out->metro_code = r->metro_code;
  }
  out->asn = v ? GeoIP_name_by_ipnum_v6(d[2], a) : GeoIP_name_by_ipnum(d[2], n);
  out->org = v ? GeoIP_name_by_ipnum_v6(d[3], a) : GeoIP_name_by_ipnum(d[3], n);
  out->isp = v ? GeoIP_name_by_ipnum_v6(d[4], a) : GeoIP_name_by_ipnum(d[4], n);
}
void drop_geo(Reply *r) {
  if (r->record)
    GeoIPRecord_delete(r->record);
  free((void *)r->asn);
  free((void *)r->org);
  free((void *)r->isp);
}
