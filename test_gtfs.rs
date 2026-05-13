use gtfs_structures::Gtfs;
fn test(gtfs: &mut Gtfs) {
    for trip in gtfs.trips.values_mut() {
        if let Some(route) = gtfs.routes.get(&trip.route_id) {
            let _ = route.long_name.as_deref();
            let _ = trip.stop_times.len();
            trip.stop_times[0].arrival_time = Some(10);
            trip.stop_times[0].departure_time = Some(10);
        }
    }
}
fn main() {}
