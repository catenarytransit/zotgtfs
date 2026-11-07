//! Residual min-cost max-flow adapted from Irvine Connect's matcher/algorithm.rs
//! at 4770b458279b9d8b338554522d9b4e5e3a8f7d50. Reverse edges let a later bus
//! displace an earlier choice, unlike greedy selection.
use super::*;
use std::collections::{BTreeMap, VecDeque};

pub(super) fn assign(mut candidates: Vec<(i16, Vec<TripMatch>)>) -> HashMap<i16, TripMatch> {
    candidates.sort_by_key(|(id, _)| *id);
    let mut trips = BTreeMap::new();
    for (_, matches) in &candidates {
        for matched in matches {
            trips.insert(matched.instance_key(), 0);
        }
    }
    for (index, value) in trips.values_mut().enumerate() {
        *value = index;
    }
    let count = candidates.len();
    let sink = count + trips.len() + 1;
    let mut flow = MinCostMaxFlow::new(sink + 1);
    for index in 0..count {
        flow.add_edge(0, index + 1, 1, 0);
    }
    for index in 0..trips.len() {
        flow.add_edge(count + index + 1, sink, 1, 0);
    }
    for (index, (_, matches)) in candidates.iter().enumerate() {
        for matched in matches {
            if !matched.score.is_finite() {
                continue;
            }
            // Scores are absolute schedule error plus geographic mismatch.
            flow.add_edge(
                index + 1,
                count + trips[&matched.instance_key()] + 1,
                1,
                (matched.score.max(0.0) * 1000.0).round() as i64,
            );
        }
    }
    // Maximize the number of assigned buses, then minimize total cost within
    // that cardinality. Independent routes form disconnected components.
    flow.solve(0, sink);
    let mut assigned = HashMap::new();
    for (index, (id, matches)) in candidates.into_iter().enumerate() {
        for edge in &flow.graph[index + 1] {
            if edge.flow > 0 && edge.to > count && edge.to < sink {
                if let Some(matched) = matches
                    .iter()
                    .find(|m| count + trips[&m.instance_key()] + 1 == edge.to)
                {
                    assigned.insert(id, matched.clone());
                }
            }
        }
    }
    assigned
}

#[derive(Clone)]
struct Edge {
    to: usize,
    capacity: i32,
    flow: i32,
    cost: i64,
    reverse_edge: usize,
}

pub(super) struct MinCostMaxFlow {
    graph: Vec<Vec<Edge>>,
}

impl MinCostMaxFlow {
    pub(super) fn new(n: usize) -> Self {
        Self {
            graph: vec![Vec::new(); n],
        }
    }

    pub(super) fn add_edge(&mut self, from: usize, to: usize, capacity: i32, cost: i64) {
        let forward_edge = Edge {
            to,
            capacity,
            flow: 0,
            cost,
            reverse_edge: self.graph[to].len(),
        };
        let backward_edge = Edge {
            to: from,
            capacity: 0,
            flow: 0,
            cost: -cost,
            reverse_edge: self.graph[from].len(),
        };
        self.graph[from].push(forward_edge);
        self.graph[to].push(backward_edge);
    }

    pub(super) fn solve(&mut self, s: usize, t: usize) -> (i32, i64) {
        let mut total_flow = 0;
        let mut min_cost = 0;
        let n = self.graph.len();

        loop {
            let mut dist = vec![i64::MAX; n];
            let mut parent = vec![None; n];
            let mut edge_from = vec![None; n];
            let mut in_queue = vec![false; n];
            let mut queue = VecDeque::new();

            dist[s] = 0;
            queue.push_back(s);
            in_queue[s] = true;

            while let Some(u) = queue.pop_front() {
                in_queue[u] = false;
                for (i, e) in self.graph[u].iter().enumerate() {
                    if e.capacity > e.flow && dist[e.to] > dist[u].saturating_add(e.cost) {
                        dist[e.to] = dist[u] + e.cost;
                        parent[e.to] = Some(u);
                        edge_from[e.to] = Some(i);
                        if !in_queue[e.to] {
                            queue.push_back(e.to);
                            in_queue[e.to] = true;
                        }
                    }
                }
            }

            if dist[t] == i64::MAX {
                break;
            }

            let mut flow = i32::MAX;
            let mut curr = t;
            while curr != s {
                let p = parent[curr].unwrap();
                let idx = edge_from[curr].unwrap();
                flow = flow.min(self.graph[p][idx].capacity - self.graph[p][idx].flow);
                curr = p;
            }

            total_flow += flow;
            min_cost += flow as i64 * dist[t];
            curr = t;
            while curr != s {
                let p = parent[curr].unwrap();
                let idx = edge_from[curr].unwrap();
                self.graph[p][idx].flow += flow;
                let rev_idx = self.graph[p][idx].reverse_edge;
                self.graph[curr][rev_idx].flow -= flow;
                curr = p;
            }
        }

        (total_flow, min_cost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn matched(id: &str, cost: f64) -> TripMatch {
        TripMatch {
            trip_id: id.into(),
            route_id: "TL-1".into(),
            start_time: 32400,
            schedule_start_time: 32400,
            score: cost,
            delay_secs: 0,
            current_stop_sequence: Some(10),
            stop_id: Some("terminal".into()),
        }
    }

    #[test]
    fn residual_edges_find_global_minimum_instead_of_greedy_choices() {
        let candidates = vec![
            (1, vec![matched("a", 1.0), matched("b", 2.0)]),
            (2, vec![matched("a", 2.0), matched("b", 100.0)]),
        ];
        for input in [candidates.clone(), candidates.into_iter().rev().collect()] {
            let result = assign(input);
            assert_eq!(result[&1].trip_id, "b");
            assert_eq!(result[&2].trip_id, "a");
            assert_eq!(result.values().map(|m| m.score).sum::<f64>(), 4.0);
        }
    }

    #[test]
    fn maximize_assignments_and_keep_route_instances_separate() {
        let mut other_route = matched("a", 1.0);
        other_route.route_id = "TL-2".into();
        let result = assign(vec![
            (1, vec![matched("a", 1.0), matched("b", 2.0)]),
            (2, vec![matched("a", 2.0)]),
            (3, vec![other_route]),
        ]);
        assert_eq!(result.len(), 3);
        assert_eq!(result[&1].trip_id, "b");
        assert_eq!(result[&2].trip_id, "a");
        assert_eq!(result[&3].route_id, "TL-2");
    }

    fn at(seconds: u32) -> u64 {
        (chrono_tz::America::Los_Angeles
            .with_ymd_and_hms(2026, 11, 2, 0, 0, 0)
            .unwrap()
            .timestamp()
            + seconds as i64) as u64
    }

    fn trip(id: &str, start: u32) -> Trip {
        Trip {
            id: id.into(),
            route_id: "TL-1".into(),
            service_id: "active".into(),
            stop_times: [
                ("terminal", 33.0, 0),
                ("out", 33.005, 120),
                ("approach", 33.01, 360),
                ("terminal", 33.0, 600),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (id, lat, offset))| StopTime {
                stop: Arc::new(gtfs_structures::Stop {
                    id: id.into(),
                    latitude: Some(lat),
                    longitude: Some(-117.0),
                    ..Default::default()
                }),
                stop_sequence: (i as u32 + 1) * 10,
                arrival_time: Some(start + offset),
                departure_time: Some(start + offset),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        }
    }

    fn timetable() -> Gtfs {
        let mut gtfs = Gtfs::default();
        for (id, start) in [("correct", 32400), ("next", 33000), ("future", 33600)] {
            gtfs.trips.insert(id.into(), trip(id, start));
        }
        gtfs.calendar_dates.insert(
            "active".into(),
            vec![gtfs_structures::CalendarDate {
                service_id: "active".into(),
                date: chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
                exception_type: gtfs_structures::Exception::Added,
            }],
        );
        gtfs
    }

    fn sample(seconds: u32, latitude: f32) -> PositionSample {
        PositionSample {
            timestamp: at(seconds),
            latitude,
            longitude: -117.0,
            heading: None,
            speed_mps: Some(5.0),
            transloc_route_id: 1,
        }
    }

    fn bus(seconds: u32, latitude: f32) -> (AnteaterExpressData, u64) {
        (
            AnteaterExpressData {
                VehicleID: 1,
                RouteID: 1,
                Name: "bus".into(),
                GroundSpeed: 18.0,
                Heading: 0.0,
                Latitude: latitude,
                Longitude: -117.0,
            },
            at(seconds),
        )
    }

    #[test]
    fn reassign_minus_twenty_and_minus_nine_minutes_without_resetting_physical_lap() {
        for (wrong, departure, expected_delay) in [("future", 32400, 0), ("next", 32460, 60)] {
            let gtfs = timetable();
            let now_secs = departure + 120;
            let mut history = VehicleHistory {
                assigned_trip_id: Some(wrong.into()),
                assigned_route_id: Some("TL-1".into()),
                assigned_start_time: trip_start_secs(&gtfs.trips[wrong]),
                current_delay_secs: -1200,
                positions: vec![sample(departure, 33.0), sample(now_secs, 33.005)],
                ..Default::default()
            };
            history.loop_tracker.departed_terminal = true;
            history.loop_tracker.lap_started_at = at(departure);
            history.loop_tracker.origin_departure_epoch = Some(at(departure));
            let (vehicle, now) = bus(now_secs, 33.005);
            let matches = select_unique_trip_matches(
                &[vehicle],
                &HashMap::from([(1, history.clone())]),
                &gtfs,
                now,
            );
            assert_eq!(matches[&1].trip_id, "correct");
            assert_eq!(matches[&1].delay_secs, expected_delay);
            update_vehicle_history_for_match(
                &mut history,
                &gtfs.trips["correct"],
                &matches[&1],
                now,
            );
            assert_eq!(history.loop_tracker.lap_started_at, at(departure));
            assert_eq!(
                history.loop_tracker.origin_departure_epoch,
                Some(at(departure))
            );
            assert!(history.loop_tracker.departed_terminal);
            assert_eq!(history.current_delay_secs, expected_delay);
        }
    }

    #[test]
    fn two_running_buses_are_corrected_to_distinct_ten_minute_departures() {
        let gtfs = timetable();
        let mut histories = HashMap::new();
        let mut vehicles = Vec::new();
        for (id, departure) in [(1, 32460), (2, 33060)] {
            let mut history = VehicleHistory {
                assigned_trip_id: Some("future".into()),
                assigned_route_id: Some("TL-1".into()),
                assigned_start_time: Some(33600),
                current_delay_secs: -1200,
                positions: vec![sample(departure, 33.0), sample(departure + 120, 33.005)],
                ..Default::default()
            };
            history.loop_tracker.departed_terminal = true;
            history.loop_tracker.lap_started_at = at(departure);
            history.loop_tracker.origin_departure_epoch = Some(at(departure));
            histories.insert(id, history);
            let (mut vehicle, _) = bus(33180, 33.005);
            vehicle.VehicleID = id;
            vehicles.push(vehicle);
        }
        let matches = select_unique_trip_matches(&vehicles, &histories, &gtfs, at(33180));
        assert_eq!(matches[&1].trip_id, "correct");
        assert_eq!(matches[&2].trip_id, "next");
        assert_eq!(matches[&1].delay_secs, 60);
        assert_eq!(matches[&2].delay_secs, 60);
    }

    #[test]
    fn origin_and_final_occurrence_have_distinct_timestamps_and_delays() {
        let trip = trip("loop", 32400);
        let positions = vec![
            sample(32460, 33.0),
            sample(32580, 33.005),
            sample(32820, 33.01),
            sample(33060, 33.0),
        ];
        let path = stop_path::match_stop_path(&positions, &trip, 32400);
        assert_eq!(path, vec![Some(0), Some(1), Some(2), Some(3)]);
        let mut history = VehicleHistory {
            positions,
            current_delay_secs: 60,
            matched_stop_sequence: Some(40),
            ..Default::default()
        };
        history.loop_tracker.origin_departure_epoch = Some(at(32460));
        history.loop_tracker.lap_started_at = at(32460);
        // Include the final occurrence in the reconstruction to inspect the
        // completed arrival, independently of terminal successor publication.
        update_stop_observations(&mut history, &trip, Some(50), 32400);
        let end = history
            .stop_observations
            .iter()
            .find(|o| o.stop_sequence == 40)
            .unwrap();
        assert_eq!(end.arrival_epoch_secs, at(33060));
        let mut matched = matched("loop", 0.0);
        matched.delay_secs = 60;
        matched.current_stop_sequence = Some(40);
        let updates = stop_time_updates_for_match(
            &trip,
            &matched,
            Some(&history),
            la_midnight_epoch_secs(at(33060)),
        );
        let origin = updates
            .iter()
            .find(|u| u.stop_sequence == Some(10))
            .unwrap();
        let end = updates
            .iter()
            .find(|u| u.stop_sequence == Some(40))
            .unwrap();
        assert_eq!(origin.stop_id, end.stop_id);
        assert_eq!(
            origin.departure.as_ref().unwrap().time,
            Some(at(32460) as i64)
        );
        assert_eq!(end.arrival.as_ref().unwrap().time, Some(at(33060) as i64));
        assert_eq!(end.arrival.as_ref().unwrap().delay, Some(60));
    }

    #[test]
    fn terminal_only_history_cannot_supply_a_final_arrival_twenty_minutes_early() {
        let trip = trip("loop", 32400);
        let positions: Vec<_> = (0..120).map(|i| sample(31200 + i * 10, 33.0)).collect();
        assert_eq!(
            stop_path::match_stop_path(&positions, &trip, 32400),
            vec![Some(0); positions.len()]
        );
        let mut history = VehicleHistory {
            positions,
            current_delay_secs: -1200,
            ..Default::default()
        };
        update_stop_observations(&mut history, &trip, Some(50), 32400);
        assert!(history.stop_observations.is_empty());
    }
}
