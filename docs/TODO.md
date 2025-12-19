- Need to come up with some more validation for OGC compliance, and look into whatever hot new MAP api specs there are
- Would like to get different output formats (geotiff, black/white png, etc)
- scope out creating an API for the grid processor so that we may use it for future EDR work
- Need a better landing page with some sample queries
- startup validation of the system should be another widget on the dashboard (did all of the requests made by the hammer
  produce an actual image?)
- getFeatureInfo should support arbitrary html output
- Need some swagger docs or something for WMS and WMTS
- we have a projection crate and also reprojecting logic in the grid processor crate
- could we implement some sort of 'use this gradient' style magic? essentially just pass a b64 string of json or
  something to provide a colormap in a get request
- mayyybe we implement that magic AI/ML super duper compression thing igor showed off, would need to render each tile
  then just compress to that b64 string, ofc this would rely on the frontend being able to render it
- grid cache section in the web dashboard needs to go away
- can we combine the crates and services folders?
- we have test_renders and hammer_Results and a bunch of others lets consolidate into the validation folder
- we have a bunch of stuff in the wms-validation folder i don't think we're using, could be expanded
- some of the scripts in the scripts folder could be moved somewhere into validation
- i guess the idea of the validation folder is to help ensure the code and system are behaving as expected
- need to come up with plan to de-crapify the codebase, look through files one by one and consolidate functionality
  where possible
- unit tests for EVERYTHING
- integration tests
- various web ui links can be cleaned up into a dropdown or something on the web dashboard
- wms-api container takes the longest to start
- satellite data still using the old grid cache i presume due to the precaching config in the models directory
- style viewing and editing web app, view current styles and how they would look on the map
- need to disable all caching and 'optmizations' to get a baseline performance metric, then apply them one by one to see
  how they impact performance
- load testing needs to simulate real user scenarios
- need to consider actually deploying this to ec2 or something
    - this will bring up a whole wormy can involving security and rate limiting and api access and tokens and shit
- implement renderer queue after we've deleted the renderer worker stuff???
    - The Renderer Worker is a background service that consumes render jobs from a Redis queue and generates PNG tiles
      for caching. It enables cache warming, prefetching, and scheduled tile rendering without blocking client requests.
    - seems neat we can implement it if we feel like it later
- implement the crazy radar diffing stuff to try and reduce bandwidth for radar loops
- some kind of mismatch where MRMS is using REFD which then causes HRRR reflectivity to not render