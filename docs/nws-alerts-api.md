# NWS Alerts (EDR collection `nws-alerts`)

Watches, warnings and advisories from api.weather.gov. The downloader polls the active-alerts feed
every minute, the ingester stores it in PostGIS (`nws_alerts`, `nws_zones`), and the EDR API serves
it as GeoJSON. Config: `config/edr/nws-alerts.yaml`; code: `services/edr-api/src/handlers/nws_alerts.rs`.

| Request | Use |
|---|---|
| `GET /edr/collections/nws-alerts/position?coords=POINT(lon lat)` | **Alerts in force at my location**, worst first (see **Order**, below) |
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
circle), at most 0.05 degrees, so a CONUS view or a continent-sized area stays small (every alert
in the country is 1-2 MB, instead of tens); `/position` returns the full geometry unless you give a
`simplify`. `simplify` is a Douglas-Peucker tolerance in degrees: the area the returned shape covers
stays within about that distance of the full-resolution one. Every part of an alert stays, however
small the alert: a part (or a whole alert, such as a tornado warning on a continent view) smaller
than the tolerance is kept, as a triangle or as the convex hull of the part, not dropped. What is
narrower than about the tolerance goes, however long it is: a hole (an island in a marine zone) is
filled in and its outline disappears, and a sliver or a spit is cut off. A tolerance of 0.00011
degrees (about 12 m) or less is the same as `0`: the full outline, rounded to the 5 decimals the
coordinates have, because at that tolerance simplifying takes out about 5% of the points or fewer
and costs several times as long as the answer would. (A map view up to 0.16 degrees wide gets that
by default.) The simplified geometry is as valid as the full one at any tolerance (see `geometry`
under [Response](#response)).
`/items` also takes `status=active|all` (with `all`, `datetime=from/to` returns history,
default window the last 24 h; superseded messages hidden unless `include_superseded=true`),
`limit` (default 500, max 2000; `"truncated": true` when cut).

**Order.** `/position`, `/items`, `/area` and `/radius` list the worst alert first. The keys, in
order, each breaking the ties of the one before:

1. CAP `severity`: Extreme, Severe, Moderate, Minor, then anything else (`Unknown`, none);
2. `significance`: `warning`, `watch`, `advisory`, `statement`, then `other`. NWS rates most watches
   Severe, like the warnings, so severity alone would list a Flood Watch re-issued a minute ago
   ahead of the Flood Warning it comes with. It is the `significance` the features carry: the first
   VTEC code's letter (W, A, Y, S), else the event's name (`... Warning`, `... Watch`,
   `... Advisory`, `... Statement`);
3. `urgency`: Immediate, Expected, Future, Past, then anything else;
4. `certainty`: Observed, Likely, Possible, Unlikely, then anything else;
5. the newest `effective` first, then the `id`, so the order is the same on every request.

An alert that has not started yet (`upcoming`) is ordered like any other. The first feature of
`/position` is the one to show in full, and the rest the ones to list. A cut made by `limit` or by
the response size limit (below) is made from the end of this order.

**Answer size.** The geometry of one list answer (`/position`, `/items`, `/area`, `/radius`) is held
to the collection's `limits.max_response_size_mb` (50 MB, `config/edr/nws-alerts.yaml`), whatever
`limit` and `simplify` say: the alerts that would take it past that are left out and `"truncated":
true` says so, also when fewer than `limit` came back. Never fewer than one alert is returned. The
alerts in force now come to about 30 MB at `simplify=0` for the whole country, so the limit is
met by asking for full resolution over a history (`status=all`, `include_superseded=true`, a wide
`datetime` interval); a bounding box, the default `simplify` or `geometry=false` keep the answer
well under it. What is left out is the end of the list: the least severe, oldest alerts.

**Errors.** A mistake in the request is a `4xx` with the JSON exception body (`type`, `title`,
`status`, `detail`; `application/json`), on every route, never a plain-text body or a `5xx`:

- a query string that does not parse: `limit=abc`, `limit=`, `simplify=abc`, a parameter given
  twice (`detail` carries the reason);
- `limit` below 1 (`limit=0`, `limit=-5`). Above the maximum it is cut to 2000, not refused;
- `simplify` negative or not a finite number (`-1`, `NaN`, `inf`, `1e999`). `0` is how to ask for no
  simplification;
- a control character (a NUL, a newline, ...) in `severity` or `event`, or in the alert id, or an
  alert id that is not UTF-8 once unescaped (`/items/%FF`);
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
  polygon covered, and not at 11:00. When several messages name the same one, the first issued is
  the one that replaces it (`superseded_by`), whatever order the feed listed them in;
- it had not ended: the poll that first no longer listed it had not happened yet. A cancellation
  (a `Cancel`, or a message whose VTEC actions are all `CAN`) is over the moment it is issued, so it
  is never in force itself, not even in the minute before the poll that stored it; the alert it
  cancels is out of force from then on, and nothing is returned in between;
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
Postgres and the request answers `503` with the usual JSON exception body (`"status": 503`, "did
not finish within the server's time limit") and `Retry-After: 5`; the server logs a warning, not an
error. The limit counts the time a read spends waiting for a lock, so a `503` is not always an
expensive query: a single point or one alert can get one while the database is busy, and the same
request a few seconds later succeeds. Try again after a short delay; a large `/area`, `/radius` or
`/items` that keeps failing is the one to make smaller or simpler. The limit applies to these reads
only, not to the ingester's writes or to the API's other collections.

**Read slots.** The database pool is shared by every collection of the API, and a read holds a
connection until Postgres has finished its statement, also when the client that asked has gone
(Postgres is not told, the statement runs on until the limit). So at most half the pool's
connections (5 of 10) run alert reads at once and the rest queue for a slot, instead of a burst of
alert requests holding every connection and starving the grids, observations and trails. The wait
for a slot counts toward the same time limit, and a request that gets none in time is the same
`503`; a read cancelled with its client keeps its slot until its statement has ended.

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
   "color":"#FFA500","headline":"Severe Thunderstorm Warning issued November 1 at 4:10PM MST until November 1 at 5:00PM MST by NWS Albuquerque NM",
   "formal_headline":"Severe Thunderstorm Warning issued November 1 at 4:10PM MST until November 1 at 5:00PM MST by NWS Albuquerque NM","area":"Bernalillo; Sandoval",
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
  MONDAY`). NWS leaves it out of some messages (new short-fused warnings, but also some Small Craft
  Advisories, Avalanche Advisories and Special Weather Statements), and then `headline` is the
  formal one (the example above: `Severe Thunderstorm Warning issued November 1 at 4:10PM MST until
  ... by NWS Albuquerque NM`), so the style can change between the messages of one `vtec_event`.
  **Day words in the broadcast text (`TODAY`, `TONIGHT`, `THIS EVENING`, `MONDAY`) are relative to
  the local day the message was `sent`**, and an alert that stays in force for a day or two keeps
  them: `GALE WARNING TONIGHT AND MONDAY` sent on Sunday at 3 AM still says "tonight" on Monday
  morning, and means Sunday night. For text that has dates, use `formal_headline`, or `onset` and
  `ends`.
- `formal_headline` is always NWS' own headline, dated, on every feature of every list and
  `/items/{id}` (for a Flood Warning: `Flood Warning issued November 1 at 7:26PM MST until November
  2 at 7:30PM MST by NWS Pueblo CO`; it is `null` when NWS sent none). It is the headline to show
  where a card has no room for the dates of `sent`, `onset` and `ends`, or where one style is
  wanted for every alert. The broadcast text is also in `parameters.NWSheadline` on `/items/{id}`.
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
  advisories do) or, for `HAZARD...`/`SOURCE...`/`IMPACT...`, bare. When NWS wrote neither `WHAT`
  nor `HAZARD`, `summary.what` is made from the rest of the text, so that such an alert has
  something to show beyond its headline (see [Alerts with no WHAT](#alerts-with-no-what)); an
  alert with no text, or one that has other labels but no `WHAT` or `HAZARD`, has no `what`.
  `/items/{id}` returns the same properties and adds, besides the top-level `meta` every answer has:
  - the untrimmed text: `description` (wraps collapsed into paragraphs, a `- ` item or a forecast
    period on a line of its own), `description_raw` and `instruction_raw` (as NWS sent them),
    `sections` (the labelled parts, the forecast `periods` and the `narrative`);
  - NWS' own fields: `message_type` (`Alert`, `Update`, `Cancel`), `category`,
    `response`, `vtec` (every parsed VTEC code), `parameters`, `ugc`, `zones` (the affected-zone
    keys), `references` (earlier messages this one replaces), `is_marine`. `parameters` is NWS'
    `parameters` object without four bulky broadcast-system keys: `BLOCKCHANNEL`, `EAS-ORG`,
    `CMAMtext` and `CMAMlongtext` (the Wireless Emergency Alert text);
  - the lifecycle: `superseded_by`, `superseded_at`, `ended_at`, `first_seen_at`, `last_seen_at`
    (the last poll that listed the alert), `updated_at` (when the content last changed).
- An empty `features` array is a valid "no alerts" answer **only when `meta.stale` is false**.
  `stale` is true when the last successful poll is over 5 minutes old (or never happened);
  `meta.last_error` appears when a poll has failed since the last success.
- Every successful response, the collection document included: `Cache-Control: public, max-age=60`.
  Errors carry no `Cache-Control` and are not cached.
- **Age of the data.** Responses are cached for up to 60 seconds at the CDN (and by browsers), so
  `timeStamp` and `meta.data_age_seconds` describe the moment the body was generated, not the moment
  you read it; `meta.stale` was decided then too. The data's true age is `meta.data_age_seconds` plus
  the time since `timeStamp`: `data_age_seconds + (now - timeStamp)`, or `now - meta.last_success_at`,
  with `now` the client's clock (both are in the body, so they work cross-origin and from a response
  that sat in the browser's cache). Use that, not the number as sent, before deciding the feed is
  fresh or that an empty answer means "no alerts". Do not use the `Age` header for this: it counts
  only the seconds the CDN held the answer, not the time it sat in the browser's own cache, so it
  understates the age of a response the browser reused. (It is exposed to cross-origin scripts, so
  `response.headers.get('Age')` works, but it is not the data's age.) The data also trails NWS by a
  poll or two (the downloader polls every 60 seconds, plus the time a poll takes), so with the cache
  an alert NWS has just issued, changed or cancelled can take about two minutes to show up.
- Alerts that NWS gave only zone codes get their geometry once the zones are fetched
  (`geometry_source: pending` until then; such an alert does not match point queries yet, or only
  where the zones fetched so far cover it).
- `geometry` (when asked for) is a `MultiPolygon`, or `null` when the alert has none yet. Its
  coordinates are rounded to 5 decimals (about a metre), and it is valid as returned (OGC simple
  features: no hole outside its shell, nested shells or self-intersection), whatever `simplify`
  did to it or the rounding would have done to a thin zone, so a client may hand it straight to a
  geometry library, also a recent one (GEOS 3.10 and later, shapely 2, JTS) that checks for nested
  shells: a part that simplification left inside another part, such as an island that a coastal
  zone's outline was simplified across, is merged into it. A part smaller than a metre is dropped.
  The rings follow the right-hand rule of RFC 7946 section 3.1.6: every exterior ring runs
  counter-clockwise and every hole clockwise, whichever way NWS drew its polygon. (Earlier versions
  returned the unions of zones with clockwise exterior rings. A client that relied on that, d3-geo
  for one, which wants clockwise exterior rings on the sphere, has to `rewind` the geometry now.)
- `geometry_source`: `polygon` (NWS sent one), `zones` (union of the alert's zones), `pending` (some
  zones not fetched yet), `none` (no usable geometry: the zones have none, or the database rejected
  the polygon and there were no zones to fall back on). A polygon the database rejects never costs
  the alert or the rest of the feed: it is stored without its own polygon and uses its zones. The
  same goes for a zone whose geometry is rejected (it is kept as `invalid_geometry` and retried
  after a day).

## Alerts with no WHAT

Most alerts write their text as `* WHAT...`, `* WHERE...`, `* WHEN...` sections, or as `HAZARD...`,
`SOURCE...` and `IMPACT...` (see `summary` above). Two kinds do not, and are over half of the
feed on a day with Alaska gales out (239 of 468 alerts on 2026-11-02 had no `WHAT` or `HAZARD`):

**Zone forecasts with periods (Alaska coastal and offshore waters: Gale Warning, Storm Warning,
Small Craft Advisory).** NWS puts the zone's forecast in the alert, one line per period, each
starting with a dot and the period in capitals:

```text
Coastal Waters Forecast for the Northern Gulf of Alaska Coast
up to 100 nm out including Kodiak Island and Cook Inlet.

Wind forecasts reflect the predominant speed and direction
expected. Sea forecasts represent an average of the highest
one-third of the combined wind wave and swell height.

.TODAY...E wind 25 kt. Seas 7 ft. Isolated thunderstorms.
.TONIGHT...SE wind 25 kt becoming E 35 kt after midnight. Seas 9
ft. Rain.
.TUE...E wind 45 kt. Seas 18 ft. Rain.
.TUE NIGHT...SE wind 50 kt. Seas 20 ft.
```

- A period header is a dot, 2 to 30 capital letters and spaces, and three dots: `.TODAY...`,
  `.TONIGHT...`, `.REST OF TODAY...`, `.TUE...`, `.TUE NIGHT...`, `.TUE AND TUE NIGHT...`,
  `.WED THROUGH THU...`. The period runs to the next header or the next blank line, with its wraps
  joined.
- `/items/{id}` returns them as `sections.periods`, in order:
  `[{"name": "TODAY", "text": "E wind 25 kt. Seas 7 ft. Isolated thunderstorms."}, {"name": "TUE NIGHT", "text": "SE wind 50 kt. Seas 20 ft."}, ...]`
  (`name` as NWS wrote it). The paragraphs around them, the zone's name and the boilerplate about
  how wind and seas are forecast, stay in `sections.narrative`. `description` has one period per
  line (`.TODAY...E wind 25 kt. ...\n.TONIGHT...SE wind ...`), where earlier versions ran them
  together into one paragraph.
- **`summary.what`** is the period or periods that the headline's day words name, each with its
  name in front, joined by a space: `GALE WARNING TONIGHT AND TUESDAY` gives `Tonight: SE wind 25
  kt becoming E 35 kt after midnight. Seas 9 ft. Rain. Tue: E wind 45 kt. Seas 18 ft. Rain.`
  - `TODAY`, `THIS MORNING` and `THIS AFTERNOON` name the `TODAY` and `REST OF TODAY` periods;
    `TONIGHT`, `LATE TONIGHT` and `THIS EVENING` name `TONIGHT`.
  - `MONDAY`, `MONDAY MORNING` and `MONDAY AFTERNOON` name `MON`; `MONDAY NIGHT` and `MONDAY
    EVENING` name `MON NIGHT` (and likewise for the other days). `GALE WARNING TUESDAY` names
    `TUE` and not `TUE NIGHT`.
  - A period that spans days is named by any of them: `TUE AND TUE NIGHT` by `TUESDAY` or `TUESDAY
    NIGHT`, `WED THROUGH THU` by `WEDNESDAY`, `THURSDAY` or `THURSDAY NIGHT`.
  - `AND` and the rest only join the day words. `THROUGH` makes a range of periods, because the
    hazard is in force from the start of the forecast to the day named, and the wind that makes it
    so is often in the first period, not the last: `SMALL CRAFT ADVISORY THROUGH MONDAY` is every
    period from the first one (the one in force now) to `MON`, and `THROUGH LATE TONIGHT` is
    `TODAY` and `TONIGHT`. `THIS EVENING THROUGH TUESDAY EVENING` is `TONIGHT`, `TUE` and `TUE
    NIGHT`: from the first period the first day names to the last period the second names, none
    in the middle skipped. `THROUGH MONDAY` ends with `MON`, not `MON NIGHT`. A range can be
    several periods long; in a list `summary.what` is cut at 500 characters.
  - With no day word, or none that a period carries (the formal headline of an alert NWS gave no
    broadcast text, or `MONDAY` where the forecast has no `MON`), `what` is the first period,
    `Today: ...` or `Rest of today: ...`.
  - The labels are short: `Tue night`, `Wed through Thu`.
  - **`Today`, `Tonight` and `Rest of today` are the day the forecast was written.** NWS writes
    them for the day it issues the forecast, and the alert stays in force after that day is over
    (a Small Craft Advisory sent on Sunday at noon, read on Monday). So `summary.what` is made
    from the time the alert is read: a period that is over (a day runs 6 AM to 6 PM, a night
    6 PM to 6 AM, in the office's time, which the zone in `formal_headline` gives) is left out when
    the headline names another that is not, or else the first period that is not over is
    taken, and a `Tonight` that is still in progress the next day is labelled with the day it meant
    (`Sun night`). If every period is over, it says the period the headline names, labelled `Sun`
    or `Sun night`. Read the same day, the labels are NWS' own. An alert whose `formal_headline`
    names no time zone is not adjusted. The text of a period is the forecast as of `sent`; it
    does not change after that.
  - Alerts stored by the first version of the parser, which has the forecast run together in one
    narrative paragraph, give the same `summary.what`: the periods are read out of that paragraph
    until the next poll rewrites them as `sections.periods`.

**Free text (Special Weather Statements, Hydrologic Outlooks, Marine Weather Statements, test
messages).** With no labels and no periods at all, `summary.what` is the first sentence of the
text, and the second too when both fit in 200 characters; a first sentence over 200 characters is cut
at a word (`…`). A paragraph that is not a sentence (it does not end in `.`, `!` or `?`, such as
the river's name that starts a Hydrologic Outlook, or a table) is passed over, and so are the two
lines that every Alaska marine forecast starts with and that say nothing about the weather (`Coastal
Waters Forecast for ...` and `Wind forecasts reflect the predominant speed and direction
expected.`). The product id line
(`ESFEAX`) is dropped, as everywhere. An alert whose text has other labels but no `WHAT` or `HAZARD`
gets no `what`.

An alert with `WHAT` or `HAZARD` never has its `summary` changed by any of this: `what` is
filled only when NWS wrote neither. `summary.what` here is derived when the answer is made, from
`sections`, so a change to these rules needs no rewrite of the stored alerts. (The periods
themselves are parsed when an alert is stored; alerts stored before they were read get them from
the rewrite that a new parser version makes at the next poll, see the lifecycle notes.)

## Lifecycle notes

- An alert is in force from issue until its end time (NWS' `ends`, else `expires`), unless NWS stops
  listing it, a newer message replaces it, or it is a cancellation (`messageType: Cancel` or a VTEC
  `CAN` action). A VTEC `EXP` message ("will expire at ...") is **not** over when it arrives: it
  stays in force until its end time.
- `superseded_at` (on `/items/{id}`) is when a replacement was issued, never later than the poll
  that first showed it; `ended_at` is the poll that stopped listing the alert, or, for a
  cancellation, the moment it was issued (also never later than the poll that first showed it).
  Both feed the `datetime` instants above. `ended_at` is **set only when NWS stops listing the
  alert** (or sends a cancellation): an alert whose end time has passed is `status: ended` from that
  instant, but its `ended_at` stays `null` until the poll that no longer lists it, usually within a
  minute or two, and a `superseded` alert that also left the feed never gets one. To say when an
  alert ended, use `ends` (else `expires`), `superseded_at` or `ended_at`, whichever applies, not
  `ended_at` alone.
- The ingester ignores a feed whose own `updated` stamp is older than the one it already applied
  (NWS is behind a CDN and a lagging node can serve a stale copy): the poll counts for
  `meta.last_success_at`, but nothing in it changes an alert, so a stale copy cannot end alerts
  issued since. Equal stamps are applied normally.
- `last_seen_at` moves with every poll, `updated_at` only when the message changes. The ingester
  does not rewrite an alert NWS has not changed (it recognises it by a hash of the NWS feature and
  of the version of its own parsing): a poll of a feed whose alerts are the same ones as the
  minute before only sets `last_seen_at`, and a change to how alerts are parsed is applied to the
  stored ones by one rewrite at the next poll, which leaves `updated_at` alone unless the message
  itself differs. A build that predates the hash (after a rollback, or while it still runs beside
  this one) rewrites every listed alert with its own parsing and moves `last_seen_at` without
  touching the hash; the ingester sees that the two have come apart and rewrites those alerts at its
  next poll, so no step is needed after a rollback. An alert that left the feed and comes back is
  live again (`ended_at` cleared) with its `first_seen_at` and `updated_at` as they were.
- Zone boundaries are fetched lazily (about 800 on a cold start) and cached: 30 days when fetched,
  24 hours before retrying a 404 or an empty/invalid zone, 15 minutes before retrying a fetch that
  failed (timeout, 5xx, rate limit). A failed refresh never discards a boundary already held. An
  alert waiting on a zone whose fetch failed stays `geometry_source: pending`.

