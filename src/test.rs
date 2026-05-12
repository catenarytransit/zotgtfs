pub fn check(gtfs: &gtfs_structures::Gtfs) {
    let _ = gtfs.trips.values().next().unwrap().frequencies.iter();
}
