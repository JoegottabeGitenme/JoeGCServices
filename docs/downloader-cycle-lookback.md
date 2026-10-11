# Downloader: missed cycles and truncated runs

## What was wrong

The forecast poll loop had two defects that together lost data every day.

1. **The poll period drifted.** `run_forever` ran a cycle and *then* slept `poll_interval_secs`, so the
   real period was `3600 s + the cycle's own run time` (5-13 min for HRRR). Poll times slid later each
   hour (17:43, 18:51, 19:41, 20:57, 22:04 ...) until an entire cycle was never polled. Measured on
   prod, Oct 5-10: HRRR lost 2-3 of 24 cycles a day; `nbm-conus` showed the same drift.
2. **Each cycle was examined once.** Discovery looked only at the single newest cycle and never came
   back. A cycle is published over time: HRRR f00 appears about 50 minutes after the cycle and f48 of
   the 00/06/12/18Z runs about 2 h 05 m after it. Whatever had not appeared at that one look was lost
   for good. The 18Z runs of Oct 8-11 kept f0-f22, f0-f22, f0-f41 and (at the time of the
   investigation) f0-f12, although NOMADS already had f48.

User-visible effect: `get_timeseries_for_feature` stitches trail conditions by `valid_time` across
runs, so a truncated extended run shrank the forecast horizon and left hourly holes. The comment there
("recent runs hold 5-41 forecast hours ... hourly HRRR runs are shorter") attributed the variance to
cycle length; most of it was this.

## What changed

* `run_on_fixed_period`: a fixed-period timer (`tokio::time::interval`, `MissedTickBehavior::Skip`).
  Work time no longer moves later polls. If a cycle outlasts a period, the tick that came due runs as
  soon as it returns and further missed ticks are dropped (no catch-up burst).
* `recent_cycles`: each poll examines the newest available cycle and `schedule.cycle_lookback`
  earlier ones (newest first, correct across midnight and year end). Default `0` = old behaviour.
* Discovery checks `completed_downloads` **before** the HEAD probe (`url` is the primary key). A
  finished cycle therefore costs no network requests and a truncated one costs only its missing hours.
* Per-cycle forecast hours: `schedule.extended_cycles` + `schedule.extended_forecast_hours`. The load
  step rejects a half-configured pair or an extended cycle that is not in `cycles`.
* Config: `hrrr` = 0-18 h, with 00/06/12/18Z extended to 48 h (measured: `t17z.wrfprsf18` exists,
  `wrfprsf19` is 404, `t18z.wrfprsf48` exists), `cycle_lookback: 3`. The five `nbm-*` models get
  `cycle_lookback: 3`.

## Not done

* NBM has three horizon tiers (from our own history for `nbm-conus`: ~f58 for most cycles, f189 for
  03/09/15Z, f264 for 06/07/12/13Z). They were not set up as `extended_cycles` because the 07Z/13Z
  assignment is unexplained; short NBM cycles still probe hours that do not exist.
* Gaps that already exist are not backfilled beyond the 3-cycle lookback, and files older than
  retention are not worth fetching.
* The status API's `files_expected` uses the base (short) hour list.
