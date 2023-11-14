## Instructions to get static data

1. Run this command to download the json data from TransLoc
```bash
cargo run --bin downloadstatic
```

2. Run this command to convert the json data into GTFS static data
```bash
cargo run --bin processstatic
```

3. Zip the folder `anteater_gtfs` with no underlying folder inside the final zip

Now your Anteater static file is done!

# Notes

UC Irvine Anteater Express Agency Number: 1039

** DO NOT UPDATE PROST VERSION!! Not backwards compatible! **

### Shapes fix

Circular routes such as the 2023 M, E, and N lines, will parse correctly through the greedy algorithm Kyler wrote.

Routes that overlap themselves or have weird sharp corners cause errors. Transloc uses the crappiest segment idea ever.

In `route-sup.json`, it may be needed to override the shape.

### Schedule Parameters

All fields mark departure times only

`!`: Cancel the stop entirely
`#`: Disable boardings
`*`: Cancel all service prior to this stop
`$`: Cancel all boardings after this stop

Leave field empty: Algorithm estimates

# Motivation

I have no life and I want this data to show up on the rest of the Catenary

### Extra notes

ASUCI is a bunch of stupid losers.