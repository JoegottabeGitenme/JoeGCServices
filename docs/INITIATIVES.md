Need to do validation of the system, not only 'Is this OGC compliant' but 'can we deliver on all the data we advertise?'

Need to look over every bit of the code and give human review, come up with tests, ask if certain parts are needed, can do benchmarks before and after to test that code behavior is the same

need to expand product offering, how does this affect caching and overall storage requirements?

Need to add pre reqeusts of every data set on ingest, so that users don't experience first hit lag

GOES needs attention

MRMS needs attention

Need a tool to visualize how a style config will look so users can tweak colors and gradients etc
Could expand this to things like wind barbs, station plots etc. Add in some functionality to add styles through a POST request from qualified users

The whole UX of the admin 'stuff' could all be in one app, need to figure out how that would look

docs need human review, does this make sense? do these commands even work? is this even correct?

once system is more stable need to do load tests with the various 'enhancements' turned on and off to see if they are actually improving anything

Once deployed to k8s, how do we manage multiple of the same types of these things? shared cache stuff, rolling updates, healthy state after startup should include a warmed cache type things

web viewer needs some more UX things so users can test things a real user might do. Time slider mostly, multiple layers

Do we allow multiple 'layers' being stacked so that we could do the rendering and stacking on the backend so the frontend doesn't have to?

profiling in github actions needs to only profile code that has changed, harder problem is 'how does this code affect other code' kinda stuff