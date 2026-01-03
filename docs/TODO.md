- Would like to get different output formats (geotiff, black/white png, etc) in WMS and WMTS
- getFeatureInfo should support arbitrary html output
- we have a projection crate and also reprojecting logic in the grid processor crate
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
- load testing needs to be cleaned up, we should have only a handlful of scenarios and just use some outside scripts to
  manage things like cold/warm cache etc.
- need to consider actually deploying this to ec2 or something
    - this will bring up a whole wormy can involving security and rate limiting and api access and tokens and shit
- implement renderer queue after we've deleted the renderer worker stuff???
    - The Renderer Worker is a background service that consumes render jobs from a Redis queue and generates PNG tiles
      for caching. It enables cache warming, prefetching, and scheduled tile rendering without blocking client requests.
    - seems neat we can implement it if we feel like it later
- implement the crazy radar diffing stuff to try and reduce bandwidth for radar loops
- need to implement custom CRS for getMap requests and WMTS we can add more projections later but for now we just have
  the two outlined in the spec
- add some sort of json schema for the various yaml files so that making new ones is less of a pain
- web viewer uses and incredible amount of memory, 1.9G just with one single layer loaded and no zooming or panning
    - this may have been due to the 10s of thousands of objects in minio
    - see if this happens again after we fixed the orphaned files issue
    - it still happens and its still happening after claude 'fixed' the memory leaks
- see about adding the 'metocean' compliance stuff in WMS/WMTS if applicable
- need to give the swagger docs a human pass to catch some of the errors
- why are we getting orphaned files constantly?
- evicting things from chunk cache seems to bring things to a crawl, need to explain how evictions work and how we're
  getting chunk cache entry count
- downloader should prioritize radar/satellite, perhaps a thread or threadpool for each data type so they don't block
  eachother
- ingester should be able to handle multiple downloads at once, currently it does 1 at a time
- README needs some screenshots and to be more accurate
- Dateline crossing loads the whole grid, this will cause requests over the Pacific to be slow
- Cache warming should just fill L2 cache
- Most chunks decompressed are 1 MB, figure out how much storage to cache common products
- Cache invalidation section in the docs doesn't quite make sense
- Cache TTL for weather data isn't the whole picture, could also invalidate when we get new data kinda thing
- System design high level architecture diagram isn't right anymore
- Minio object storage section is wrong
- download and ingest new data https://vlab.noaa.gov/web/mdl/ndfd-grid-data
- also NBM https://vlab.noaa.gov/web/mdl/nbm-download
- looping radar (only product that does this) absolutely eats up the chunk cache
- let's try to get registered on the OGC implementation database
- add some security scanning as another docker compose image that can we enabled optionally, this will show a webpage
  that will run some of the various security scanners and display some results
- need to check units and other metadata in all query type outputs
- lets come up with some kind of visualier for the EDR api that can show off the current collections and some data on a map
- lets make the styles of the compliance checking web pages consistent, maybe one page for all compliance checking, one page for all coverage checking
- need to update prometheus and grafana and loki and stuff for the EDR api still
- need to check units in WMS now that we fixed units being passed into zarr format during ingestion
- let's remove the minikube stuff from the start.sh script