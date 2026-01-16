- Would like to get different output formats (geotiff, black/white png, etc) in WMS and WMTS
- getFeatureInfo should support arbitrary html output
- could we implement some sort of 'use this gradient' style magic? essentially just pass a b64 string of json or
  something to provide a colormap in a get request
- mayyybe we implement that magic AI/ML super duper compression thing igor showed off, would need to render each tile
  then just compress to that b64 string, ofc this would rely on the frontend being able to render it
    - this could be useful for a mobile app
- various web ui links can be cleaned up into a dropdown or something on the web dashboard
- style viewing and editing web app, view current styles and how they would look on the map
- need to disable all caching and 'optimizations' to get a baseline performance metric, then apply them one by one to
  see
  how they impact performance
- implement renderer queue after we've deleted the renderer worker stuff???
    - The Renderer Worker is a background service that consumes render jobs from a Redis queue and generates PNG tiles
      for caching. It enables cache warming, prefetching, and scheduled tile rendering without blocking client requests.
    - seems neat we can implement it if we feel like it later
- implement the crazy radar diffing stuff to try and reduce bandwidth for radar loops
- need to implement custom CRS for getMap requests and WMTS we can add more projections later but for now we just have
  the two outlined in the spec
- web viewer uses and incredible amount of memory, 1.9G just with one single layer loaded and no zooming or panning
    - it still happens and its still happening after claude 'fixed' the memory leaks
- see about adding the 'metocean' compliance stuff in WMS/WMTS if applicable (probs not needed)
- why are we getting orphaned files constantly?
- evicting things from chunk cache seems to bring things to a crawl, need to explain how evictions work and how we're
  getting chunk cache entry count
- downloader should prioritize radar/satellite, perhaps a thread or threadpool for each data type so they don't block
  eachother
- Dateline crossing loads the whole grid, this will cause requests over the Pacific to be slow
- Cache warming should just fill L2 cache
- Documentation stuff:
  - Cache invalidation section in the docs doesn't quite make sense
  - Cache TTL for weather data isn't the whole picture, could also invalidate when we get new data kinda thing
  - System design high level architecture diagram isn't right anymore
  - Minio object storage section is wrong
- let's try to get registered on the OGC implementation database
- need to check units and other metadata in all query type outputs
- lets come up with some kind of visualier for the EDR api that can show off the current collections and some data on a map
- lets make the styles of the compliance checking web pages consistent, maybe one page for all compliance checking, one page for all coverage checking
- need to update prometheus and grafana and loki and stuff for the EDR api still
- need to check units in WMS now that we fixed units being passed into zarr format during ingestion
- create robots.txt with goodies
- I think it's worth downsampling MRMS data to a more reasonable resolution even after the pyramid fixes
- need to see if precaching and cache warming is actually doing anything, more important after we trim down datasets
- lets download WMS and WMTS compliance documents and ensure our services adhere
- need to double check getFeatureInfo for all WMS products within their BBOX's, should be fairly easy to script up
- need to greatly reduce the amount of data ingested and figure out exactly what we want as far as app needs so we aren't carrying bloat
  - radar only need past 30 minutes
  - sat only past 30 minutes (do we even need satellite?)
  - hrrr only support 12 hour forecasts so let's do some math on when we can get it, and download only 'future hours'
  - gfs kinda pointless
  - NBM/NDFD mayyyybe
  - various models dont need all these different products right now
- Downloading and ingesting should not bring services to a crawl so we may want to figure out core usage
- at this point I wouldn't mind trading out more ram usage to save cpu usage, so let's fill up and increase our caches and make sure memory limits are strict for the various services
- update response size limits in EDR, a full globe png of GFS isn't actually that much data
- scope out 'specialized' PNGs for EDR where we encode multiple values
- scope out 'specialized' enpoints for the various activities and the data required for them
- add resolution stuff for trajectory and corridor queries
- figure out how to ingest ensemble data and it's members and then expose pre-computed stuff through WMS/EDR
- use the .idx files in s3 to only download the parameters we want from NOAA so we don't download the entire grib file every time