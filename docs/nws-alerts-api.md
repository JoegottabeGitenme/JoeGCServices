# NWS Alerts (EDR collection `nws-alerts`)

Watches, warnings and advisories from api.weather.gov. The downloader polls the active-alerts feed
every minute, the ingester stores it in PostGIS (`nws_alerts`, `nws_zones`), and the EDR API serves
it as GeoJSON. Config: `config/edr/nws-alerts.yaml`; code: `services/edr-api/src/handlers/nws_alerts.rs`.

| Request | Use |
|---|---|
| `GET /edr/collections/nws-alerts/position?coords=POINT(lon lat)` | **Alerts in force at my location**, worst first |
| `GET /edr/collections/nws-alerts/items?bbox=w,s,e,n` | Alerts in the map view (geometry on by default) |
| `GET /edr/collections/nws-alerts/items/{id}` | One alert in full (id is a URN; raw or `%3A`-encoded) |
| `GET /edr/collections/nws-alerts/area?coords=POLYGON((...))` | Alerts intersecting a polygon |
| `GET /edr/collections/nws-alerts/radius?coords=POINT(lon lat)&within=50&within-units=km` | Alerts within a distance (max 500 km) |
| `GET /edr/collections/nws-alerts` | Collection document (data age in the description) |

Common parameters: `severity=Severe,Extreme`, `event=Tornado Warning,Flash Flood Warning` (comma
lists, case-insensitive), `geometry=true|false` (default: false on position/area/radius, true on
items), `datetime=<instant>` (the alerts in force at that moment; default now, see
[`datetime` instants](#datetime-instants)), `simplify=<degrees>` on
`/items` (`0` disables; default scales with the bbox so a CONUS view stays small).
`/items` also takes `status=active|all` (with `all`, `datetime=from/to` returns history,
default window the last 24 h; superseded messages hidden unless `include_superseded=true`),
`limit` (default 500, max 2000; `"truncated": true` when cut).

### `datetime` instants

`datetime=<instant>` on `/position`, `/area`, `/radius` and `/items` (`status=active`) answers
"what was in force at that moment", from each message's own history rather than from today's
flags. A message is returned when, at the instant:

- it had been issued (`effective`, else `sent`, at or before the instant);
- no newer message had replaced it. An update, or the cancellation of the alert, replaces it from
  the moment that message was issued, not from the later poll that noticed: a warning replaced at
  11:00 by an update with a smaller polygon is still returned at 10:59 for a point only its old
  polygon covered, and not at 11:00;
- it had not ended: the poll that first no longer listed it had not happened yet;
- its end time (`ends`, else `expires`) had not been reached. The end is exclusive: an alert
  ending at 18:00 is not in force at 18:00.

So an alert issued after the instant is never returned, and one that was in force then is returned
even if it has been replaced or has ended since.

Each feature's `status` is read at the instant too: `upcoming` while its onset (else effective) is
still ahead of it, otherwise `active`. `ended` and `superseded` do not occur, since a message that
had ended or been replaced by then is not returned. `include_superseded` does not apply to
`status=active`; `status=all` with an instant is the same window and the same labels, but hides a
message since replaced unless `include_superseded=true`.

No `datetime`, or `datetime=now`, is the live set: the alerts in force right now, read from the
current lifecycle flags (the fast path the apps use). A future instant returns what is in force
now and will still be at that instant; replacements NWS has not issued yet are not known.

**History is as deep as retention.** An alert is kept for 7 days after it ended, expired or was
replaced (`NWS_ALERTS_RETENTION_HOURS` on the ingester, default 168, minimum 24). An older instant
is not an error: it returns the alerts still stored that were in force then, so the answer is
incomplete, and empty once everything of that time has been deleted. `meta` describes the feed's
freshness now, not at the instant.

**`/area` polygons.** `coords` is one `POLYGON((lon lat, lon lat, ...))` ring in either winding
order (lon -180..180, lat -90..90; the closing vertex is optional). Rings PostGIS would have to
repair are refused with a `400` and the reason instead of being tried, because repairing a ring
that crosses itself costs the database seconds to minutes, so the ring must:

- have at most 500 vertices (the repeat of the first one that closes it is not counted) and not
  all lie on one line;
- be simple: no two edges cross, touch or overlap each other (a bowtie, a vertex on another edge,
  two lobes sharing a vertex and an edge doubling back over the previous one are all refused;
  collinear edges that stay apart are fine);
- have no inner rings (holes are not supported; subtract the hole on the client);
- have no edge spanning more than 180 degrees of longitude, so a polygon over the antimeridian is
  refused: split it into two requests, one with longitudes up to 180 and one from -180.

The polygon is read as planar lon/lat. Text after the closing parentheses is a `400` too.

**Query time limit.** Every database read behind these endpoints is capped at
`NWS_ALERTS_QUERY_TIMEOUT_MS` milliseconds (an edr-api environment variable, default `10000`; a
value that is not a positive number is ignored). A query that outlasts it is cancelled by
Postgres and the request answers `503` with the usual JSON exception body (`"status": 503`,
"too expensive and timed out"); the server logs a warning, not an error. Retrying the same request
will not help: ask for a smaller or simpler area. The limit applies to these reads only, not to
the ingester's writes or to the API's other collections.

**Test alerts.** NWS' feed also carries messages whose CAP status is not `Actual` (`Test`,
`Exercise`, `System`, `Draft`; e.g. the periodic "Test Message"). They are all stored, but every
list query (`/position`, `/items`, `/area`, `/radius`, history windows included) returns only
`Actual` ones unless you pass `include_non_actual=true`. A message with no status at all counts as
`Actual`. `/items/{id}` returns the alert whatever its status. Each feature says which it is in
`cap_status`.

## Response

A FeatureCollection with `numberReturned`, `timeStamp` and `meta`:

```json
{"type":"FeatureCollection","numberReturned":1,"timeStamp":"2026-11-01T23:30:05Z",
 "meta":{"source":"api.weather.gov","last_success_at":"2026-11-01T23:29:41Z","data_age_seconds":24,"stale":false},
 "features":[{"type":"Feature","id":"urn:oid:2.49.0.1.840.0.937dcb...001.1","geometry":null,
  "properties":{"event":"Severe Thunderstorm Warning","severity":"Severe","urgency":"Immediate","certainty":"Observed",
   "color":"#FFA500","headline":"Severe Thunderstorm Warning issued ... until 5:00 PM MST","area":"Bernalillo; Sandoval",
   "sender":"NWS Albuquerque NM","sent":"...","effective":"...","onset":"...","ends":"2026-11-02T00:00:00Z","expires":"...",
   "status":"active","cap_status":"Actual","significance":"warning","action":"NEW","vtec_event":"KABQ.SV.W.0428",
   "summary":{"hazard":"60 mph wind gusts.","source":"Radar indicated.","impact":"Expect damage to roofs, siding, and trees."},
   "instruction":"For your protection move to an interior room ...","geometry_source":"polygon",
   "links":[{"rel":"self","href":".../collections/nws-alerts/items/urn%3Aoid%3A2.49.0.1.840.0.937dcb...001.1"},{"rel":"alternate","href":"https://api.weather.gov/alerts/urn:oid:..."}]}}]}
```

- `status`: `active`, `upcoming` (issued, hazard starts later: label it "Starts 6 PM"), `ended`,
  `superseded` (a newer message replaced it; only seen via `/items/{id}` or `include_superseded`).
  As of now, or as of the request's `datetime` instant (see above). This is the lifecycle status;
  the CAP one (`Actual`, `Test`, ...) is `cap_status`.
- `color` is the NWS hazard-map colour of the event (`Test Message` has the map's Test colour,
  `#F0FFFF`); an event the map does not list gets one by `significance`.
- `significance`: `warning` / `watch` / `advisory` / `statement` / `other`. Group updates of one
  event by `vtec_event` (NEW, CON, EXT, UPG share it).
- `summary` has only the keys present (`what`, `where`, `when`, `impacts`, `hazard`, `source`,
  `impact`), trimmed to 500 characters; `instruction` to 600 (`…`). They are read from the labelled
  lines of the NWS text, written `* WHAT...`, `*WHAT...` (no space, as the Colorado avalanche
  advisories do) or, for `HAZARD...`/`SOURCE...`/`IMPACT...`, bare. Alerts whose text has no such
  labels (Alaska coastal-water forecasts, old-style watches) have an empty `summary`; their text is
  in `sections.narrative` and `description` on `/items/{id}`. `/items/{id}` adds the untrimmed
  text, `description`, `description_raw`, `instruction_raw`, `sections`, `vtec`, `parameters`, `ugc`,
  `zones`, `references`, `superseded_by`, `superseded_at`, `ended_at`, `first_seen_at`,
  `last_seen_at`.
- An empty `features` array is a valid "no alerts" answer **only when `meta.stale` is false**.
  `stale` is true when the last successful poll is over 5 minutes old (or never happened);
  `meta.last_error` appears when a poll has failed since the last success.
- Every response: `Cache-Control: public, max-age=60`.
- Alerts that NWS gave only zone codes get their geometry once the zones are fetched
  (`geometry_source: pending` until then; such an alert does not match point queries yet, or only
  where the zones fetched so far cover it).
- `geometry_source`: `polygon` (NWS sent one), `zones` (union of the alert's zones), `pending` (some
  zones not fetched yet), `none` (no usable geometry: the zones have none, or the database rejected
  the polygon and there were no zones to fall back on). A polygon the database rejects never costs
  the alert or the rest of the feed: it is stored without its own polygon and uses its zones. The
  same goes for a zone whose geometry is rejected (it is kept as `invalid_geometry` and retried
  after a day).

## Lifecycle notes

- An alert is in force from issue until its end time (`ends`, else `expires`), unless NWS stops
  listing it, a newer message replaces it, or it is a cancellation (`messageType: Cancel` or a VTEC
  `CAN` action). A VTEC `EXP` message ("will expire at ...") is **not** over when it arrives: it
  stays in force until its end time.
- `superseded_at` (on `/items/{id}`) is when a replacement was issued, never later than the poll
  that first showed it; `ended_at` is the poll that stopped listing the alert. Both feed the
  `datetime` instants above.
- The ingester ignores a feed whose own `updated` stamp is older than the one it already applied
  (NWS is behind a CDN and a lagging node can serve a stale copy): the poll counts for
  `meta.last_success_at`, but nothing in it changes an alert, so a stale copy cannot end alerts
  issued since. Equal stamps are applied normally.
- Zone boundaries are fetched lazily (about 800 on a cold start) and cached: 30 days when fetched,
  24 hours before retrying a 404 or an empty/invalid zone, 15 minutes before retrying a fetch that
  failed (timeout, 5xx, rate limit). A failed refresh never discards a boundary already held. An
  alert waiting on a zone whose fetch failed stays `geometry_source: pending`.

