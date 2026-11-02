# NWS Alerts (EDR collection `nws-alerts`)

Watches, warnings and advisories from api.weather.gov. The downloader polls the active-alerts feed
every minute, the ingester stores it in PostGIS (`nws_alerts`, `nws_zones`), and the EDR API serves
it as GeoJSON. Config: `config/edr/nws-alerts.yaml`; code: `services/edr-api/src/handlers/nws_alerts.rs`.

| Request | Use |
|---|---|
| `GET /edr/collections/nws-alerts/position?coords=POINT(lon lat)` | **Alerts in force at my location**, worst first |
| `GET /edr/collections/nws-alerts/items?bbox=w,s,e,n` | Alerts in the map view (geometry on by default); `bbox=170,50,-170,60` (west > east) crosses the antimeridian |
| `GET /edr/collections/nws-alerts/items/{id}` | One alert in full (id is a URN; raw or `%3A`-encoded) |
| `GET /edr/collections/nws-alerts/area?coords=POLYGON((...))` | Alerts intersecting a polygon |
| `GET /edr/collections/nws-alerts/radius?coords=POINT(lon lat)&within=50&within-units=km` | Alerts within a distance (max 500 km) |
| `GET /edr/collections/nws-alerts` | Collection document (live data age in the description) |

**Collection document.** It advertises what the endpoints return and nothing else: every query
(`position`, `area`, `radius`) and the collection itself list only `application/geo+json` (and the
`GeoJSON` short form); there is no CoverageJSON. `links` has `self`, `collection`, `root` and
`items` (`/collections/nws-alerts/items`, the alerts); the generic `/locations` and `/instances`
links other collections carry are left out because they lead nowhere here. The `description` ends
with the live data age (`Data age 24 s.`, or `STALE: ...` once the feed is over 5 minutes old),
computed for each request. The cached `/edr/collections` list carries no age, because a relative age
in a cached body is wrong by the time it is read: use the collection document or any answer's
`meta`.

**Output format.** Every query answers `application/geo+json`, and only that. `f=GeoJSON` (also
`geojson`, `application/geo+json`, and plain `json`) is accepted; any other `f` (`CoverageJSON`,
`png`, `zzz`) is a `400` that says GeoJSON is what is served, the same answer an unknown `f` gets on
the grid collections. Without `f`, the `Accept` header has to admit `application/geo+json` or
`application/json` (`*/*` does): `Accept: application/vnd.cov+json` alone is a `406 Not
Acceptable`, as the grid collections answer an `Accept` they cannot serve. An `f` overrides
`Accept`.

Common parameters: `severity=Severe,Extreme`, `event=Tornado Warning,Flash Flood Warning` (comma
lists, case-insensitive), `geometry=true|false` (default: false on position/area/radius, true on
items), `datetime=<instant>` (the alerts in force at that moment; default now, see
[`datetime` instants](#datetime-instants)), `simplify=<degrees>` (the geometry tolerance, `0`
disables). Without `simplify`, `/items`, `/area` and `/radius` simplify the geometry they return to
about a pixel of a 1500 px wide view of the query's extent (the bbox, the polygon's bounding box, the
circle), at most 0.05 degrees, so a CONUS view or a continent-sized area stays small (a few hundred
KB for every alert in the country, instead of tens of MB); `/position` returns the full geometry
unless you give a `simplify`.
`/items` also takes `status=active|all` (with `all`, `datetime=from/to` returns history,
default window the last 24 h; superseded messages hidden unless `include_superseded=true`),
`limit` (default 500, max 2000; `"truncated": true` when cut).

**Errors.** A mistake in the request is a `4xx` with the JSON exception body (`type`, `title`,
`status`, `detail`; `application/json`), on every route, never a plain-text body or a `5xx`:

- a query string that does not parse: `limit=abc`, `limit=`, `simplify=abc`, a parameter given
  twice (`detail` carries the reason);
- `limit` below 1 (`limit=0`, `limit=-5`). Above the maximum it is cut to 2000, not refused;
- `simplify` negative or not a finite number (`-1`, `NaN`, `inf`, `1e999`). `0` is how to ask for no
  simplification;
- a control character (a NUL, a newline, ...) in `severity` or `event`, or in the alert id;
- a `datetime` interval that ends before it starts.

A `+` in a UTC offset has to be sent as `%2B` (`datetime=2026-11-01T18:00:00%2B00:00`); a bare `+`
reaches the server as a space, and the datetime is refused.

### The antimeridian

NWS draws a zone that touches 180 as two polygons, one on each side (the Bering Sea offshore zones
are 171E to 180 and 180 to 171W), and an alert for both is a MultiPolygon with a part each side.
So queries that span the line find alerts on both sides:

- `/items?bbox=w,s,e,n` with **west greater than east** is a box across the antimeridian, as in
  OGC API Features: `bbox=170,50,-170,60` is the 20 degree wide band from 170E over the line to
  170W (not a 340 degree one). It is the two boxes `170..180` and `-180..-170`; an alert with a part
  in each is returned once. A box with no width (`west == east`) or no height is a `400`.
- `/radius` measures geodetic distance, so `coords=POINT(179.95 57.5)&within=50` returns the alerts
  3 km away on the far side of the line as well as the ones at the centre, from either side
  (`180` and `-180` are the same place), and a circle that reaches a pole sees every longitude.
- `/area` still reads its polygon as planar lon/lat and refuses one that crosses the line (see
  below): use `bbox` or `/radius` for a dateline area, or two `/area` requests, one each side, and
  join the answers by `id`.

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
- its end time (NWS' `ends`, else `expires`) had not been reached. The end is exclusive: an alert
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
   "color":"#FFA500","headline":"Severe Thunderstorm Warning issued November 1 at 4:10PM MST until November 1 at 5:00PM MST by NWS Albuquerque NM","area":"Bernalillo; Sandoval",
   "sender":"NWS Albuquerque NM","sent":"...","effective":"...","onset":"...","ends":"2026-11-02T00:00:00Z","expires":"...",
   "status":"active","cap_status":"Actual","significance":"warning","action":"NEW","vtec_event":"KABQ.SV.W.0428",
   "summary":{"hazard":"60 mph wind gusts.","source":"Radar indicated.","impact":"Expect damage to roofs, siding, and trees."},
   "instruction":"For your protection move to an interior room ...","geometry_source":"polygon",
   "links":[{"rel":"self","href":".../collections/nws-alerts/items/urn%3Aoid%3A2.49.0.1.840.0.937dcb...001.1"},{"rel":"alternate","href":"https://api.weather.gov/alerts/urn:oid:..."}]}}]}
```

- `ends` is the end time NWS gave the hazard, `null` when it gave none (an "until further notice"
  Flood Warning, the Colorado avalanche advisories, some statements). `expires` is when the message
  itself expires and is always there. An alert with a null `ends` is in force until NWS re-issues
  it or stops listing it, and is treated as over at `expires` if neither happens first (its
  `status` becomes `ended` then): show `expires` as the latest it can last, not as a forecast end
  of the hazard.
- `headline` is the text NWS broadcasts for the alert (its `NWSheadline` parameter) when it gives one,
  otherwise NWS' own headline. Most alerts have the broadcast text, upper case, in local time
  without a date and up to 200 characters (`FLOOD WARNING REMAINS IN EFFECT UNTIL 730 PM MST
  MONDAY`); a new short-fused warning has none, so it gets the formal one (the example above:
  `Severe Thunderstorm Warning issued November 1 at 4:10PM MST until ... by NWS Albuquerque NM`),
  and the style can change between the messages of one `vtec_event`. `/items/{id}` also returns
  `formal_headline`, always NWS' own headline (for a Flood Warning: `Flood Warning issued
  November 1 at 7:26PM MST until November 2 at 7:30PM MST by NWS Pueblo CO`), for a client that
  wants one style or the issue and end times; the broadcast text is also in
  `parameters.NWSheadline`.
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
  in `sections.narrative` and `description` on `/items/{id}`. `/items/{id}` returns the same
  properties and adds, besides the top-level `meta` every answer has:
  - the untrimmed text: `description` (wraps collapsed into paragraphs), `description_raw` and
    `instruction_raw` (as NWS sent them), `sections` (the labelled parts and the `narrative`);
  - NWS' own fields: `formal_headline`, `message_type` (`Alert`, `Update`, `Cancel`), `category`,
    `response`, `vtec` (every parsed VTEC code), `parameters`, `ugc`, `zones` (the affected-zone
    keys), `references` (earlier messages this one replaces), `is_marine`. `parameters` is NWS'
    `parameters` object without four bulky broadcast-system keys: `BLOCKCHANNEL`, `EAS-ORG`,
    `CMAMtext` and `CMAMlongtext` (the Wireless Emergency Alert text);
  - the lifecycle: `superseded_by`, `superseded_at`, `ended_at`, `first_seen_at`, `last_seen_at`,
    `updated_at` (when the content last changed).
- An empty `features` array is a valid "no alerts" answer **only when `meta.stale` is false**.
  `stale` is true when the last successful poll is over 5 minutes old (or never happened);
  `meta.last_error` appears when a poll has failed since the last success.
- Every successful response, the collection document included: `Cache-Control: public, max-age=60`.
  Errors carry no `Cache-Control` and are not cached.
- **Age of the data.** Responses are cached for up to 60 seconds at the CDN (and by browsers), so
  `timeStamp` and `meta.data_age_seconds` describe the moment the body was generated, not the moment
  you read it; `meta.stale` was decided then too. The data's true age is `data_age_seconds` plus the
  time since `timeStamp` (or the `Age` header), or `now - meta.last_success_at` on your own clock:
  use that, not the number as sent, before deciding the feed is fresh or that an empty answer means
  "no alerts". The data also trails NWS by a poll or two (the downloader polls every 60 seconds,
  plus the time a poll takes), so with the cache an alert NWS has just issued, changed or cancelled
  can take about two minutes to show up.
- Alerts that NWS gave only zone codes get their geometry once the zones are fetched
  (`geometry_source: pending` until then; such an alert does not match point queries yet, or only
  where the zones fetched so far cover it).
- `geometry` (when asked for) is a `MultiPolygon`, or `null` when the alert has none yet. Its
  coordinates are rounded to 5 decimals (about a metre), and it is valid as returned (OGC simple
  features: no hole outside its shell, nested shells or self-intersection), whatever `simplify`
  did to it or the rounding would have done to a thin zone, so a client may hand it straight to a
  geometry library. A part smaller than a metre is dropped.
- `geometry_source`: `polygon` (NWS sent one), `zones` (union of the alert's zones), `pending` (some
  zones not fetched yet), `none` (no usable geometry: the zones have none, or the database rejected
  the polygon and there were no zones to fall back on). A polygon the database rejects never costs
  the alert or the rest of the feed: it is stored without its own polygon and uses its zones. The
  same goes for a zone whose geometry is rejected (it is kept as `invalid_geometry` and retried
  after a day).

## Lifecycle notes

- An alert is in force from issue until its end time (NWS' `ends`, else `expires`), unless NWS stops
  listing it, a newer message replaces it, or it is a cancellation (`messageType: Cancel` or a VTEC
  `CAN` action). A VTEC `EXP` message ("will expire at ...") is **not** over when it arrives: it
  stays in force until its end time.
- `superseded_at` (on `/items/{id}`) is when a replacement was issued, never later than the poll
  that first showed it; `ended_at` is the poll that stopped listing the alert. Both feed the
  `datetime` instants above. `ended_at` is **set only when NWS stops listing the alert** (or sends
  a cancellation): an alert whose end time has passed is `status: ended` from that instant, but its
  `ended_at` stays `null` until the poll that no longer lists it, usually within a minute or two,
  and a `superseded` alert that also left the feed never gets one. To say when an alert ended, use
  `ends` (else `expires`), `superseded_at` or `ended_at`, whichever applies, not `ended_at` alone.
- The ingester ignores a feed whose own `updated` stamp is older than the one it already applied
  (NWS is behind a CDN and a lagging node can serve a stale copy): the poll counts for
  `meta.last_success_at`, but nothing in it changes an alert, so a stale copy cannot end alerts
  issued since. Equal stamps are applied normally.
- Zone boundaries are fetched lazily (about 800 on a cold start) and cached: 30 days when fetched,
  24 hours before retrying a 404 or an empty/invalid zone, 15 minutes before retrying a fetch that
  failed (timeout, 5xx, rate limit). A failed refresh never discards a boundary already held. An
  alert waiting on a zone whose fetch failed stays `geometry_source: pending`.

