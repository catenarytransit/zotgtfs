# Loop trip assignment

Each vehicle has a persisted loop tracker in `vehicle_history.json`. Existing
history files remain readable because the tracker defaults to an empty state.

After the vehicle leaves the terminal by more than 100 metres, its trip identity
is retained. A return is recognized after it has been observed within 85 metres
of the penultimate stop, then within 60 metres of the terminal with a finite,
non-negative `GroundSpeed` below 1 km/h for at least 10 seconds. Stop occurrences
are distinguished by their pattern index, independently of the schedule-based
first/last-stop match. A startup position already stopped at a loop origin can
also establish a waiting state without claiming an observed completed loop.

On confirmed return, candidate successors must run on the current Los Angeles
service date, use the same route and terminal, start after the completed trip,
and depart between the observed arrival and 30 minutes afterward. The earliest
unclaimed departure is preferred. Already committed running and waiting trips
are reserved first. Template block IDs are not used: generated trips inherit
template metadata and do not establish reliable vehicle blocks. If no successor
is available, the matcher leaves the vehicle without a trip instead of selecting
the completed trip again.

As soon as a successor is assigned, both VehiclePosition and TripUpdate use its
trip ID. The vehicle is at that trip's first stop. While waiting, the trip delay
and every arrival/departure delay are zero, with the scheduled event timestamps.
This is the requested on-time policy even if the bus remains parked past its
scheduled departure; the clock alone does not advance it to another trip.

The waiting trip remains assigned through the layover and short restarts.
Leaving the 100-metre exit radius with speed at least 1 km/h ends the layover.
Normal predictions resume and the origin departure is recorded at that exit
observation. The previous loop's positions remain available for diagnostics but
are excluded from the new lap's matching and stop observations.

A route change, an observation gap exceeding two minutes, or a Los Angeles date
change resets the tracker. The generated service currently ends before midnight;
this uses the application's existing current-date service model, not a search
over adjacent GTFS service dates.

The source API has no observation timestamp. Ten-second confirmation is elapsed
time after geographic approach evidence, not a count of the 100-ms polls; frozen
upstream positions cannot be independently proven fresh. The distance thresholds,
approach-stop coverage and successor choice should be calibrated against live
vehicle recordings. Missing the penultimate-stop observation can prevent a
confirmed handover; the matcher does not invent completion from schedule time.

Run the offline fixture tests with `cargo test --locked --all-targets`.

## Stop occurrences on out-and-back streets

GPS positions are decoded together using a forward-only Viterbi path over the
trip's ordered stop occurrences, adapted from
[Irvine Connect's matcher](https://github.com/catenarytransit/irvine-connect-realtime-gtfs/blob/4770b458279b9d8b338554522d9b4e5e3a8f7d50/src/matcher/viterbi.rs).
Spatial emissions use a 25-metre Gaussian; transitions compare cumulative
stop-to-stop distance with GPS displacement (200-metre scale) and penalize
inconsistent movement direction. Moving vehicles' valid headings also help
resolve a first observation on overlapping legs. Schedule deviation has a
15-minute tolerance before a soft penalty, allowing delayed buses to remain on
the correct leg. All stop occurrences are considered, including mid-trip starts;
forward skips remain possible when observations are sparse.

For a street served in the order 1, 2, 3, 4, 5, 6, with opposing pairs 1/6,
2/5 and 3/4, an outbound position can no longer independently select a return
stop just because its coordinates or scheduled time are closer. Current-stop
identity and delay use the decoded path. Completed-stop observations are rebuilt
using only samples assigned to that particular occurrence, preventing an
outbound pass from supplying a return-stop arrival. Existing lap boundaries
still exclude positions from the preceding trip.

The matcher uses stop-to-stop distances rather than full road geometry. A
stationary first observation without a usable heading can remain ambiguous
until subsequent movement supplies direction evidence.
