- Would like to get different output formats (geotiff, black/white png, etc)
- scope out creating an API for the grid processor so that we may use it for future EDR work
- Need a better landing page with some sample queries
- getFeatureInfo should support arbitrary html output
- Need some swagger docs or something for WMS and WMTS
- we have a projection crate and also reprojecting logic in the grid processor crate
- could we implement some sort of 'use this gradient' style magic? essentially just pass a b64 string of json or
  something to provide a colormap in a get request
- mayyybe we implement that magic AI/ML super duper compression thing igor showed off, would need to render each tile
  then just compress to that b64 string, ofc this would rely on the frontend being able to render it
    - this could be useful for a mobile app
- we have test_renders and hammer_Results and a bunch of others lets consolidate into the validation folder
- some of the scripts in the scripts folder could be moved somewhere into validation
- unit tests for EVERYTHING
- integration tests
- various web ui links can be cleaned up into a dropdown or something on the web dashboard
- wms-api container takes the longest to start
- style viewing and editing web app, view current styles and how they would look on the map
- need to disable all caching and 'optimizations' to get a baseline performance metric, then apply them one by one to
  see
  how they impact performance
- load testing needs to simulate real user scenarios
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
- see about adding the 'metocean' compliance stuff in WMS/WMTS if applicable
- need to give the swagger docs a human pass to catch some of the errors