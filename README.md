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

# Motivation

I have no life and I want this data to show up on the rest of the Catenary aka Kyler's Transit Map ecosystem, along with routing.