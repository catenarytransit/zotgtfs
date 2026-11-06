//! Forward-only Viterbi matching, adapted from Irvine Connect's
//! src/matcher/viterbi.rs (4770b458279b9d8b338554522d9b4e5e3a8f7d50).
//! States are stop occurrences, so opposite sides of an out-and-back street
//! remain distinct even when their GPS geofences overlap.
use super::*;

const GPS_SIGMA_M: f64 = 25.0;
const TRANSITION_BETA_M: f64 = 200.0;

fn coords(st: &StopTime) -> Option<(f64, f64)> {
    Some((st.stop.latitude?, st.stop.longitude?))
        .filter(|(lat, lon)| lat.is_finite() && lon.is_finite())
}

fn direction_cost(a: (f64, f64), b: (f64, f64)) -> f64 {
    let magnitude = a.0.hypot(a.1) * b.0.hypot(b.1);
    if magnitude < 1e-12 {
        return 0.0;
    }
    let cosine = ((a.0 * b.0 + a.1 * b.1) / magnitude).clamp(-1.0, 1.0);
    -((1.0 + cosine) / 2.0).powi(2).max(0.01).ln()
}

fn vector(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((b.1 - a.1) * a.0.to_radians().cos(), b.0 - a.0)
}

/// Returns one occurrence index per sample. A path may start mid-trip and skip
/// stops during sparse sampling, but cannot move backwards in stop order.
pub(super) fn match_stop_path(
    samples: &[PositionSample],
    trip: &Trip,
    candidate_start: u32,
) -> Vec<Option<usize>> {
    let n = trip.stop_times.len();
    let empty = || vec![None; samples.len()];
    let Some(base_start) = trip_start_secs(trip) else {
        return empty();
    };
    if samples.is_empty() || n == 0 {
        return empty();
    }
    let coordinates: Vec<_> = trip.stop_times.iter().map(coords).collect();
    let mut cumulative = vec![0.0; n];
    for j in 1..n {
        cumulative[j] = cumulative[j - 1]
            + match (coordinates[j - 1], coordinates[j]) {
                (Some(a), Some(b)) => haversine_m(a.0, a.1, b.0, b.1),
                _ => 0.0,
            };
    }
    let mut previous = vec![f64::INFINITY; n];
    let mut back = vec![usize::MAX; samples.len() * n];
    for (t, sample) in samples.iter().enumerate() {
        let mut current = vec![f64::INFINITY; n];
        let position = (sample.latitude as f64, sample.longitude as f64);
        if !position.0.is_finite() || !position.1.is_finite() {
            return empty();
        }
        for j in 0..n {
            let (Some(stop), Some(schedule)) =
                (coordinates[j], stop_time_secs(&trip.stop_times[j]))
            else {
                continue;
            };
            let distance = haversine_m(position.0, position.1, stop.0, stop.1);
            let schedule = candidate_start as i32 + schedule as i32 - base_start as i32;
            let observed = align_service_seconds(
                la_seconds_past_midnight_from_epoch(sample.timestamp) as i32,
                schedule,
            );
            // As in Irvine Connect, allow 15 minutes of schedule deviation
            // before applying a soft temporal penalty. Delay must not select
            // the wrong side of a street simply to fit the timetable.
            let excess = ((observed - schedule).abs() as f64 - 900.0).max(0.0);
            let mut emission = (distance.powi(2) / (2.0 * GPS_SIGMA_M.powi(2))).min(50.0)
                + excess.powi(2) / (2.0 * 1200.0_f64.powi(2));
            // Heading resolves a cold start on overlapping route legs. Ignore
            // stopped/invalid headings, which do not establish travel direction.
            if let (Some(speed), Some(heading)) = (sample.speed_mps, sample.heading) {
                if speed.is_finite()
                    && speed >= 1.0
                    && heading.is_finite()
                    && (0.0..360.0).contains(&heading)
                {
                    let tangent = coordinates
                        .get(j + 1)
                        .copied()
                        .flatten()
                        .map(|next| vector(stop, next))
                        .or_else(|| {
                            j.checked_sub(1)
                                .and_then(|i| coordinates[i])
                                .map(|prev| vector(prev, stop))
                        });
                    if let Some(tangent) = tangent {
                        let radians = (heading as f64).to_radians();
                        emission += direction_cost((radians.sin(), radians.cos()), tangent);
                    }
                }
            }
            if t == 0 {
                current[j] = emission;
                continue;
            }
            let last = &samples[t - 1];
            let last_position = (last.latitude as f64, last.longitude as f64);
            let displacement =
                haversine_m(last_position.0, last_position.1, position.0, position.1);
            for i in 0..=j {
                let route_distance = cumulative[j] - cumulative[i];
                let mut transition =
                    ((route_distance - displacement).abs() / TRANSITION_BETA_M).min(50.0);
                // A tiny GPS jitter does not establish a velocity vector.
                if i != j && displacement >= 10.0 {
                    if let Some(from) = coordinates[i] {
                        transition +=
                            direction_cost(vector(last_position, position), vector(from, stop));
                    }
                }
                let cost = previous[i] + transition + emission;
                if cost < current[j] {
                    current[j] = cost;
                    back[t * n + j] = i;
                }
            }
        }
        previous = current;
    }
    let Some((mut j, cost)) = previous
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
    else {
        return empty();
    };
    if !cost.is_finite() {
        return empty();
    }
    let mut path = empty();
    for t in (0..samples.len()).rev() {
        path[t] = Some(j);
        if t > 0 {
            j = back[t * n + j];
            if j == usize::MAX {
                return empty();
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: u32) -> u64 {
        (chrono_tz::America::Los_Angeles
            .with_ymd_and_hms(2026, 11, 2, 0, 0, 0)
            .unwrap()
            .timestamp()
            + seconds as i64) as u64
    }

    fn trip() -> Trip {
        Trip {
            id: "dead-end".into(),
            route_id: "TL-1".into(),
            stop_times: [33.001, 33.002, 33.003, 33.003, 33.002, 33.001]
                .into_iter()
                .enumerate()
                .map(|(i, latitude)| StopTime {
                    stop: Arc::new(gtfs_structures::Stop {
                        id: format!("stop-{}", i + 1),
                        latitude: Some(latitude),
                        longitude: Some(if i < 3 { -117.0 } else { -117.0001 }),
                        ..Default::default()
                    }),
                    // Deliberately nonconsecutive GTFS sequences.
                    stop_sequence: (i as u32 + 1) * 10,
                    arrival_time: Some(32400 + i as u32 * 120),
                    departure_time: Some(32400 + i as u32 * 120),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn samples() -> Vec<PositionSample> {
        [33.001, 33.002, 33.003, 33.003, 33.002, 33.001]
            .into_iter()
            .enumerate()
            .map(|(i, latitude)| PositionSample {
                timestamp: at(33000 + i as u32 * 120),
                latitude,
                // GPS is closer to the opposite side on both passes.
                longitude: if i < 3 { -117.00006 } else { -117.00004 },
                speed_mps: Some(5.0),
                heading: Some(if i < 3 { 0.0 } else { 180.0 }),
                transloc_route_id: 1,
            })
            .collect()
    }

    #[test]
    fn delayed_bus_follows_both_sides_in_order_despite_gps_bias() {
        let trip = trip();
        let samples = samples();
        for end in 1..=6 {
            let path = match_stop_path(&samples[..end], &trip, 32400);
            assert_eq!(path, (0..end).map(Some).collect::<Vec<_>>());
        }
    }

    #[test]
    fn direction_resolves_a_cold_start_on_either_side() {
        let trip = trip();
        let samples = samples();
        for i in [0, 1, 4, 5] {
            assert_eq!(
                match_stop_path(&samples[i..=i], &trip, 32400),
                vec![Some(i)]
            );
        }
    }

    #[test]
    fn movement_history_resolves_direction_without_a_heading() {
        let trip = trip();
        let mut samples = samples();
        for sample in &mut samples {
            sample.heading = None;
        }
        assert_eq!(
            match_stop_path(&samples[..2], &trip, 32400),
            vec![Some(0), Some(1)]
        );
        let path = match_stop_path(&samples, &trip, 32400);
        assert_eq!(path.last(), Some(&Some(5)));
        assert!(path[..3].iter().all(|index| index.is_some_and(|i| i < 3)));
    }

    #[test]
    fn stopped_jitter_does_not_jump_to_the_return_leg() {
        let trip = trip();
        let mut samples = samples()[..2].to_vec();
        for offset in 1..=10 {
            let mut sample = samples[1].clone();
            sample.timestamp += offset * 10;
            sample.speed_mps = Some(0.0);
            sample.heading = Some(180.0);
            sample.longitude = -117.00009;
            samples.push(sample);
        }
        assert!(
            match_stop_path(&samples, &trip, 32400)
                .iter()
                .all(|index| index.is_some_and(|i| i <= 1))
        );
    }

    #[test]
    fn candidate_and_recorded_observations_use_the_same_occurrences() {
        let trip = trip();
        let samples = samples();
        let mut history = VehicleHistory {
            positions: samples[..2].to_vec(),
            ..Default::default()
        };
        let vehicle = AnteaterExpressData {
            GroundSpeed: 18.0,
            Heading: 0.0,
            Latitude: samples[1].latitude,
            Longitude: samples[1].longitude,
            Name: "test".into(),
            RouteID: 1,
            VehicleID: 1,
        };
        let candidate = CandidateTrip {
            trip_id: trip.id.clone(),
            route_id: trip.route_id.clone(),
            start_time: 32400,
            schedule_start_time: 32400,
        };
        let matched = score_candidate_trip(
            &candidate,
            &trip,
            Some(&history),
            &vehicle,
            samples[1].timestamp,
        )
        .unwrap();
        assert_eq!(matched.current_stop_sequence, Some(20));
        assert_eq!(matched.stop_id.as_deref(), Some("stop-2"));
        assert_eq!(matched.delay_secs, 600);
        // Even if an older persisted current sequence was wrong, outbound GPS
        // samples must not supply completed observations for stops 4/5/6.
        update_stop_observations(&mut history, &trip, Some(60), 32400);
        assert!(
            history
                .stop_observations
                .iter()
                .all(|o| o.stop_sequence <= 20)
        );
        history.positions = samples.clone();
        update_stop_observations(&mut history, &trip, Some(60), 32400);
        assert_eq!(
            history
                .stop_observations
                .iter()
                .map(|o| o.stop_sequence)
                .collect::<Vec<_>>(),
            vec![20, 30, 40, 50]
        );
        for observation in &history.stop_observations {
            let index = observation.stop_sequence as usize / 10 - 1;
            assert_eq!(observation.arrival_epoch_secs, samples[index].timestamp);
        }
    }

    #[test]
    fn missing_coordinates_and_sparse_history_are_supported() {
        let mut trip = trip();
        let samples = samples();
        let path = match_stop_path(&[samples[0].clone(), samples[4].clone()], &trip, 32400);
        assert_eq!(path, vec![Some(0), Some(4)]);
        trip.stop_times[1].stop = Arc::new(gtfs_structures::Stop::default());
        let path = match_stop_path(&samples, &trip, 32400);
        assert!(path.iter().all(|p| *p != Some(1)));
        assert!(path.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(match_stop_path(&[], &trip, 32400).is_empty());
        assert_eq!(
            match_stop_path(&samples, &Trip::default(), 32400),
            vec![None; 6]
        );
    }
}
