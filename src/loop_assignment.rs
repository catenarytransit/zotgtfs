//! Loop boundaries are physical events, not a change in the closest schedule.
use super::*;

const TERMINAL_ENTER_M: f64 = 60.0;
const TERMINAL_EXIT_M: f64 = 100.0;
const STOPPED_KMH: f32 = 1.0;
const CONFIRM_STOP_SECS: u64 = 10;
const MAX_NEXT_TRIP_WAIT_SECS: u32 = 30 * 60;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct LoopTracker {
    pub service_date: Option<String>,
    pub lap_started_at: u64,
    pub departed_terminal: bool,
    approach_index: usize,
    stop_candidate_since: Option<u64>,
    pub terminal: Option<TerminalVisit>,
    pub origin_departure_epoch: Option<u64>,
}

impl LoopTracker {
    pub(super) fn begin_assignment(&mut self, now: u64) {
        self.lap_started_at = now;
        self.origin_departure_epoch = None;
        if self.terminal.is_none() {
            self.departed_terminal = false;
            self.approach_index = 0;
            self.stop_candidate_since = None;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TerminalVisit {
    route_id: String,
    stop_id: String,
    latitude: f64,
    longitude: f64,
    arrival_epoch: u64,
    completed_trip_id: Option<String>,
    completed_start: Option<u32>,
}

pub(super) fn trip_runs_today(gtfs: &Gtfs, trip: &Trip, now: u64) -> bool {
    let date = la_time(now).date();
    if let Some(exception) = gtfs.calendar_dates.get(&trip.service_id).and_then(|dates| {
        dates
            .iter()
            .find(|exception| exception.date.to_string().parse::<Date>().ok() == Some(date))
    }) {
        return exception.exception_type == gtfs_structures::Exception::Added;
    }
    gtfs.calendar.get(&trip.service_id).is_some_and(|calendar| {
        if date < calendar.start_date.to_string().parse::<Date>().unwrap()
            || date > calendar.end_date.to_string().parse::<Date>().unwrap()
        {
            return false;
        }
        match date.weekday() {
            jiff::civil::Weekday::Monday => calendar.monday,
            jiff::civil::Weekday::Tuesday => calendar.tuesday,
            jiff::civil::Weekday::Wednesday => calendar.wednesday,
            jiff::civil::Weekday::Thursday => calendar.thursday,
            jiff::civil::Weekday::Friday => calendar.friday,
            jiff::civil::Weekday::Saturday => calendar.saturday,
            jiff::civil::Weekday::Sunday => calendar.sunday,
        }
    })
}

fn stopped(vehicle: &AnteaterExpressData) -> bool {
    vehicle.GroundSpeed.is_finite() && (0.0..STOPPED_KMH).contains(&vehicle.GroundSpeed)
}

fn distance_to(vehicle: &AnteaterExpressData, stop: &gtfs_structures::Stop) -> Option<f64> {
    Some(haversine_m(
        vehicle.Latitude as f64,
        vehicle.Longitude as f64,
        stop.latitude?,
        stop.longitude?,
    ))
}

fn loop_origin(trip: &Trip) -> Option<&StopTime> {
    let first = trip.stop_times.first()?;
    let last = trip.stop_times.last()?;
    (trip.stop_times.len() >= 3 && first.stop.id == last.stop.id).then_some(first)
}

fn terminal_visit(trip: &Trip, now: u64, completed: bool) -> Option<TerminalVisit> {
    let first = loop_origin(trip)?;
    Some(TerminalVisit {
        route_id: trip.route_id.clone(),
        stop_id: first.stop.id.clone(),
        latitude: first.stop.latitude?,
        longitude: first.stop.longitude?,
        arrival_epoch: now,
        completed_trip_id: completed.then(|| trip.id.clone()),
        completed_start: completed.then(|| trip_start_secs(trip)).flatten(),
    })
}

/// Called before matching, so a stopped return can publish the successor in the
/// same update. The terminal visit survives assignment changes and restarts.
pub(super) fn prepare_loop_assignment(
    history: &mut VehicleHistory,
    vehicle: &AnteaterExpressData,
    gtfs: &Gtfs,
    now: u64,
) {
    if !vehicle.Latitude.is_finite() || !vehicle.Longitude.is_finite() {
        return;
    }
    if let Some(visit) = &history.loop_tracker.terminal {
        let distance = haversine_m(
            vehicle.Latitude as f64,
            vehicle.Longitude as f64,
            visit.latitude,
            visit.longitude,
        );
        if distance > TERMINAL_EXIT_M
            && vehicle.GroundSpeed.is_finite()
            && vehicle.GroundSpeed >= STOPPED_KMH
        {
            history.loop_tracker.terminal = None;
            history.loop_tracker.departed_terminal = true;
            history.loop_tracker.approach_index = 0;
            history.loop_tracker.stop_candidate_since = None;
            history.loop_tracker.lap_started_at = now;
            history.loop_tracker.origin_departure_epoch = Some(now);
            history.current_delay_secs = history
                .assigned_start_time
                .map(|start| observed_delay_secs(now, start))
                .unwrap_or(0);
        }
        return;
    }

    let assigned = history
        .assigned_trip_id
        .as_ref()
        .and_then(|id| gtfs.trips.get(id));
    if let Some(trip) = assigned {
        let Some(origin) = loop_origin(trip) else {
            return;
        };
        let Some(distance) = distance_to(vehicle, &origin.stop) else {
            return;
        };
        if history.loop_tracker.lap_started_at == 0
            && !history.loop_tracker.departed_terminal
            && distance <= TERMINAL_ENTER_M
            && stopped(vehicle)
        {
            // Upgrade a legacy history saved while parked. Without a tracker
            // there is no evidence of a completed lap, so reacquire an upcoming
            // departure instead of pinning an old schedule-based assignment.
            history.loop_tracker.terminal = terminal_visit(trip, now, false);
            history.loop_tracker.lap_started_at = now;
            history.assigned_trip_id = None;
            history.assigned_route_id = None;
            history.assigned_start_time = None;
            history.stop_observations.clear();
            return;
        }
        if distance > TERMINAL_EXIT_M {
            history.loop_tracker.departed_terminal = true;
            // The approach is a stop occurrence on the decoded lap, not an
            // independent nearest stop across a dead-end street.
            let samples: Vec<_> = history
                .positions
                .iter()
                .filter(|sample| sample.timestamp >= history.loop_tracker.lap_started_at)
                .cloned()
                .collect();
            let path =
                stop_path::match_stop_path(&samples, trip, trip_start_secs(trip).unwrap_or(0));
            if let Some(index) = path.last().copied().flatten() {
                if index > 0
                    && index + 1 < trip.stop_times.len()
                    && distance_to(vehicle, &trip.stop_times[index].stop)
                        .is_some_and(|d| d <= STOP_OBSERVATION_RADIUS_M)
                {
                    history.loop_tracker.approach_index =
                        history.loop_tracker.approach_index.max(index);
                }
            }
        }
        let returned = history.loop_tracker.departed_terminal
            && history.loop_tracker.approach_index == trip.stop_times.len() - 2;
        if returned && distance <= TERMINAL_ENTER_M && stopped(vehicle) {
            let since = *history.loop_tracker.stop_candidate_since.get_or_insert(now);
            // The upstream API has no observation timestamp. This is elapsed
            // confirmation after an observed approach, not a poll counter.
            if now.saturating_sub(since) >= CONFIRM_STOP_SECS {
                history.loop_tracker.terminal = terminal_visit(trip, since, true);
                history.loop_tracker.lap_started_at = now;
                history.loop_tracker.origin_departure_epoch = None;
                history.stop_observations.clear();
            }
        } else {
            history.loop_tracker.stop_candidate_since = None;
        }
        return;
    }

    // A process can first see a vehicle already waiting at its origin. This
    // establishes a layover without pretending that we observed a whole lap.
    if stopped(vehicle) {
        let service_secs = la_seconds_past_midnight_from_epoch(now);
        let origin_trip = gtfs
            .trips
            .values()
            .filter(|trip| {
                route_id_matches(vehicle.RouteID, &trip.route_id)
                    && trip_runs_today(gtfs, trip, now)
            })
            .filter(|trip| {
                trip_start_secs(trip).is_some_and(|start| {
                    start >= service_secs && start <= service_secs + MAX_NEXT_TRIP_WAIT_SECS
                })
            })
            .filter(|trip| {
                loop_origin(trip)
                    .and_then(|st| distance_to(vehicle, &st.stop))
                    .is_some_and(|d| d <= TERMINAL_ENTER_M)
            })
            .min_by_key(|trip| (trip_start_secs(trip), &trip.id));
        if let Some(trip) = origin_trip {
            history.loop_tracker.terminal = terminal_visit(trip, now, false);
            history.loop_tracker.lap_started_at = now;
        }
    }
}

/// Some(empty) deliberately prevents fallback to the completed loop when no
/// eligible successor exists. Departure times are anchored to arrival, so the
/// clock moving during a layover never makes the bus advance another trip.
pub(super) fn terminal_trip_matches(
    vehicle: &AnteaterExpressData,
    history: &VehicleHistory,
    gtfs: &Gtfs,
    now: u64,
) -> Option<Vec<TripMatch>> {
    let visit = history.loop_tracker.terminal.as_ref()?;
    let arrival_secs = la_seconds_past_midnight_from_epoch(visit.arrival_epoch);
    let mut matches = gtfs
        .trips
        .values()
        .filter(|trip| {
            trip.route_id == visit.route_id
                && route_id_matches(vehicle.RouteID, &trip.route_id)
                && trip_runs_today(gtfs, trip, now)
        })
        .filter(|trip| loop_origin(trip).is_some_and(|st| st.stop.id == visit.stop_id))
        .filter_map(|trip| {
            let start = trip_start_secs(trip)?;
            if visit
                .completed_start
                .is_some_and(|completed| start <= completed)
                || visit.completed_trip_id.as_deref() == Some(trip.id.as_str())
                || start < arrival_secs
                || start > arrival_secs + MAX_NEXT_TRIP_WAIT_SECS
            {
                return None;
            }
            let origin = trip.stop_times.first()?;
            Some(TripMatch {
                trip_id: trip.id.clone(),
                route_id: trip.route_id.clone(),
                start_time: start,
                schedule_start_time: start,
                score: (start - arrival_secs) as f64 / 45.0,
                delay_secs: 0,
                current_stop_sequence: Some(origin.stop_sequence),
                stop_id: Some(origin.stop.id.clone()),
            })
        })
        .collect::<Vec<_>>();
    // Once the successor is committed, it is the only candidate for this visit.
    if let Some(id) = &history.assigned_trip_id {
        if visit.completed_trip_id.as_ref() != Some(id) {
            matches.retain(|m| &m.trip_id == id);
        }
    }
    matches.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then_with(|| a.trip_id.cmp(&b.trip_id))
    });
    Some(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u32) -> u64 {
        (jiff::civil::date(2026, 11, 2)
            .at(0, 0, 0, 0)
            .to_zoned(crate::timezone::get("America/Los_Angeles"))
            .unwrap()
            .timestamp()
            .as_second()
            + seconds as i64) as u64
    }

    fn trip(id: &str, start: u32) -> Trip {
        let stops = [
            ("terminal", 33.0, 0),
            ("outbound", 33.005, 120),
            ("approach", 33.01, 360),
            ("terminal", 33.0, 600),
        ];
        Trip {
            id: id.into(),
            route_id: "TL-1".into(),
            service_id: "active".into(),
            stop_times: stops
                .into_iter()
                .enumerate()
                .map(|(i, (id, lat, offset))| StopTime {
                    stop: Arc::new(gtfs_structures::Stop {
                        id: id.into(),
                        latitude: Some(lat),
                        longitude: Some(-117.0),
                        ..Default::default()
                    }),
                    stop_sequence: (i * 10) as u32,
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
        for (id, start) in [
            ("old", 32400),
            ("other_bus", 32640),
            ("next", 33120),
            ("later", 33600),
        ] {
            gtfs.trips.insert(id.into(), trip(id, start));
        }
        gtfs.calendar_dates.insert(
            "active".into(),
            vec![gtfs_structures::CalendarDate {
                service_id: "active".into(),
                date: "2026-11-02".parse().unwrap(),
                exception_type: gtfs_structures::Exception::Added,
            }],
        );
        gtfs
    }

    fn bus(id: i16, latitude: f32, speed: f32) -> AnteaterExpressData {
        AnteaterExpressData {
            VehicleID: id,
            RouteID: 1,
            Name: format!("Bus {id}"),
            Latitude: latitude,
            Longitude: -117.0,
            GroundSpeed: speed,
            Heading: 0.0,
        }
    }

    fn running_history() -> VehicleHistory {
        VehicleHistory {
            assigned_trip_id: Some("old".into()),
            assigned_route_id: Some("TL-1".into()),
            assigned_start_time: Some(32400),
            current_delay_secs: 180,
            ..Default::default()
        }
    }

    fn observe(history: &mut VehicleHistory, bus: &AnteaterExpressData, gtfs: &Gtfs, seconds: u32) {
        append_position_sample(history, bus, at(seconds));
        prepare_loop_assignment(history, bus, gtfs, at(seconds));
    }

    fn returned_history(gtfs: &Gtfs) -> VehicleHistory {
        let mut history = running_history();
        observe(&mut history, &bus(1, 33.005, 15.0), gtfs, 32520);
        observe(&mut history, &bus(1, 33.01, 15.0), gtfs, 32630);
        observe(&mut history, &bus(1, 33.0, 0.0), gtfs, 32740);
        assert!(history.loop_tracker.terminal.is_none());
        observe(&mut history, &bus(1, 33.0, 0.0), gtfs, 32750);
        assert!(history.loop_tracker.terminal.is_some());
        history
    }

    #[test]
    fn completed_loop_publishes_next_trip_on_time_and_holds_through_layover() {
        let gtfs = timetable();
        let mut history = returned_history(&gtfs);
        let mut vehicle = bus(1, 33.0, 0.0);
        // Two upstream polls in the same second do not count as confirmation.
        let next = terminal_trip_matches(&vehicle, &history, &gtfs, at(32750))
            .unwrap()
            .remove(0);
        assert_eq!(next.trip_id, "next"); // other_bus started before this arrival
        update_vehicle_history_for_match(&mut history, &gtfs.trips["next"], &next, at(32750));
        assert_eq!(history.current_delay_secs, 0);
        assert_eq!(history.matched_stop_sequence, Some(0));
        assert!(history.stop_observations.is_empty());
        let updates = stop_time_updates_for_match(
            &gtfs.trips["next"],
            &next,
            Some(&history),
            la_midnight_epoch_secs(at(32750)),
        );
        assert_eq!(updates.len(), 4);
        for (update, scheduled) in updates.iter().zip(&gtfs.trips["next"].stop_times) {
            assert_eq!(update.arrival.as_ref().unwrap().delay, Some(0));
            assert_eq!(update.departure.as_ref().unwrap().delay, Some(0));
            assert_eq!(
                update.departure.as_ref().unwrap().time,
                Some(la_midnight_epoch_secs(at(32750)) + scheduled.departure_time.unwrap() as i64)
            );
        }
        // Persisted state must hold the same trip even after its scheduled start
        // and the following departure. Keep feeding observations without a gap.
        history = serde_json::from_str(&serde_json::to_string(&history).unwrap()).unwrap();
        for seconds in (32760..=33720).step_by(60) {
            observe(&mut history, &vehicle, &gtfs, seconds);
            let matches = rank_trip_matches_for_vehicle(
                &vehicle,
                Some(&history),
                &gtfs,
                at(seconds),
                seconds,
            );
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].trip_id, "next");
        }
        vehicle.Latitude = 33.005;
        vehicle.GroundSpeed = 15.0;
        observe(&mut history, &vehicle, &gtfs, 33730);
        assert!(history.loop_tracker.terminal.is_none());
        assert_eq!(history.loop_tracker.origin_departure_epoch, Some(at(33730)));
        let updates = stop_time_updates_for_match(
            &gtfs.trips["next"],
            &next,
            Some(&history),
            la_midnight_epoch_secs(at(33730)),
        );
        assert_eq!(updates[0].departure.as_ref().unwrap().delay, Some(610));
    }

    #[test]
    fn departure_without_final_approach_and_brief_or_fast_terminal_passes_do_not_complete() {
        let gtfs = timetable();
        let mut history = running_history();
        observe(&mut history, &bus(1, 33.005, 15.0), &gtfs, 32520);
        observe(&mut history, &bus(1, 33.0, 0.0), &gtfs, 32530);
        observe(&mut history, &bus(1, 33.0, 0.0), &gtfs, 32550);
        assert!(history.loop_tracker.terminal.is_none());
        observe(&mut history, &bus(1, 33.01, 15.0), &gtfs, 32630);
        for speed in [1.0, 5.0, -1.0, f32::NAN] {
            observe(&mut history, &bus(1, 33.0, speed), &gtfs, 32640);
            assert!(history.loop_tracker.terminal.is_none());
        }
        observe(&mut history, &bus(1, 33.0, 0.0), &gtfs, 32640);
        for _ in 0..100 {
            observe(&mut history, &bus(1, 33.0, 0.0), &gtfs, 32640);
        }
        assert!(history.loop_tracker.terminal.is_none());
        observe(&mut history, &bus(1, 33.001, 10.0), &gtfs, 32645);
        assert!(history.loop_tracker.terminal.is_none());
        assert!(history.loop_tracker.stop_candidate_since.is_none());
    }

    #[test]
    fn startup_at_terminal_assigns_upcoming_trip_without_claiming_a_completed_lap() {
        let gtfs = timetable();
        let vehicle = bus(1, 33.0, 0.5);
        let mut history = VehicleHistory::default();
        observe(&mut history, &vehicle, &gtfs, 32700);
        assert!(
            history
                .loop_tracker
                .terminal
                .as_ref()
                .unwrap()
                .completed_trip_id
                .is_none()
        );
        let matches = terminal_trip_matches(&vehicle, &history, &gtfs, at(32700)).unwrap();
        assert_eq!(matches[0].trip_id, "next");
        // Stopped GPS jitter cannot be mistaken for departure.
        observe(&mut history, &bus(1, 33.001, 0.0), &gtfs, 32710);
        assert!(history.loop_tracker.terminal.is_some());
    }

    #[test]
    fn no_successor_does_not_reassign_completed_trip() {
        let mut gtfs = timetable();
        let history = returned_history(&gtfs);
        gtfs.trips.retain(|id, _| id == "old" || id == "other_bus");
        let ranked = rank_trip_matches_for_vehicle(
            &bus(1, 33.0, 0.0),
            Some(&history),
            &gtfs,
            at(32750),
            32750,
        );
        assert!(ranked.is_empty());
    }

    #[test]
    fn active_calendar_and_exceptions_filter_all_candidates() {
        let mut gtfs = timetable();
        let mut inactive = trip("inactive", 33000);
        inactive.service_id = "inactive".into();
        gtfs.trips.insert(inactive.id.clone(), inactive);
        assert!(
            !candidate_trip_instances(1, &gtfs, 32750, at(32750))
                .iter()
                .any(|c| c.trip_id == "inactive")
        );
        gtfs.calendar_dates.get_mut("active").unwrap()[0].exception_type =
            gtfs_structures::Exception::Deleted;
        assert!(candidate_trip_instances(1, &gtfs, 32750, at(32750)).is_empty());
        assert!(!trip_runs_today(
            &gtfs,
            &gtfs.trips["old"],
            at(32750) + 86400
        ));
    }

    #[test]
    fn committed_waiting_trips_are_reserved_before_new_terminal_assignments() {
        let gtfs = timetable();
        let mut incumbent = returned_history(&gtfs);
        let next = terminal_trip_matches(&bus(2, 33.0, 0.0), &incumbent, &gtfs, at(32750))
            .unwrap()
            .remove(0);
        update_vehicle_history_for_match(&mut incumbent, &gtfs.trips["next"], &next, at(32750));
        let histories = HashMap::from([(1, returned_history(&gtfs)), (2, incumbent)]);
        let matches = select_unique_trip_matches(
            &[bus(1, 33.0, 0.0), bus(2, 33.0, 0.0)],
            &histories,
            &gtfs,
            at(32750),
        );
        assert_eq!(matches[&2].trip_id, "next");
        assert_eq!(matches[&1].trip_id, "later");
    }

    #[test]
    fn route_change_outage_and_service_day_change_clear_layover() {
        let gtfs = timetable();
        for (route, timestamp) in [(2, at(32760)), (1, at(33000)), (1, at(32760) + 86400)] {
            let mut history = returned_history(&gtfs);
            let mut vehicle = bus(1, 33.0, 0.0);
            vehicle.RouteID = route;
            append_position_sample(&mut history, &vehicle, timestamp);
            assert!(history.loop_tracker.terminal.is_none());
            assert!(history.assigned_trip_id.is_none());
        }
    }

    #[test]
    fn end_to_end_matching_hands_over_once_and_rearms_after_departure() {
        let gtfs = timetable();
        let mut histories = HashMap::from([(1, running_history())]);
        // The very early next trip is corrected to other_bus after departure;
        // the physical lap still completes once and selects later on return.
        for (seconds, latitude, speed, expected) in [
            (32520, 33.005, 15.0, "old"),
            (32630, 33.01, 15.0, "old"),
            (32740, 33.0, 0.0, "old"),
            (32750, 33.0, 0.0, "next"),
            (32820, 33.0, 0.0, "next"),
            (32930, 33.005, 15.0, "other_bus"),
            (33040, 33.01, 15.0, "other_bus"),
            (33150, 33.0, 0.0, "other_bus"),
            (33160, 33.0, 0.0, "later"),
        ] {
            let vehicle = bus(1, latitude, speed);
            observe(histories.get_mut(&1).unwrap(), &vehicle, &gtfs, seconds);
            let matches = select_unique_trip_matches(&[vehicle], &histories, &gtfs, at(seconds));
            assert_eq!(matches[&1].trip_id, expected);
            update_vehicle_history_for_match(
                histories.get_mut(&1).unwrap(),
                &gtfs.trips[expected],
                &matches[&1],
                at(seconds),
            );
        }
    }

    #[test]
    fn legacy_history_and_weekday_calendars_remain_supported() {
        let mut serialized = serde_json::to_value(running_history()).unwrap();
        serialized.as_object_mut().unwrap().remove("loop_tracker");
        let mut legacy: VehicleHistory = serde_json::from_value(serialized).unwrap();
        assert!(legacy.loop_tracker.terminal.is_none());
        let mut gtfs = timetable();
        observe(&mut legacy, &bus(1, 33.0, 0.0), &gtfs, 32750);
        let matches = terminal_trip_matches(&bus(1, 33.0, 0.0), &legacy, &gtfs, at(32750)).unwrap();
        assert_eq!(matches[0].trip_id, "next");
        gtfs.calendar_dates.clear();
        let date = "2026-11-02".parse().unwrap();
        gtfs.calendar.insert(
            "active".into(),
            gtfs_structures::Calendar {
                id: "active".into(),
                start_date: date,
                end_date: date,
                monday: true,
                tuesday: false,
                wednesday: false,
                thursday: false,
                friday: false,
                saturday: false,
                sunday: false,
            },
        );
        assert!(trip_runs_today(&gtfs, &gtfs.trips["old"], at(32750)));
        gtfs.calendar.get_mut("active").unwrap().monday = false;
        assert!(!trip_runs_today(&gtfs, &gtfs.trips["old"], at(32750)));
    }

    #[test]
    fn previous_lap_positions_cannot_supply_new_stop_observations() {
        let gtfs = timetable();
        let mut history = returned_history(&gtfs);
        let next = terminal_trip_matches(&bus(1, 33.0, 0.0), &history, &gtfs, at(32750))
            .unwrap()
            .remove(0);
        update_vehicle_history_for_match(&mut history, &gtfs.trips["next"], &next, at(32750));
        // A point from the old loop, spatially and temporally plausible for the
        // new trip, must still be rejected by the physical handover boundary.
        history.positions.push(PositionSample {
            timestamp: at(32740),
            latitude: 33.005,
            longitude: -117.0,
            speed_mps: Some(0.0),
            heading: None,
            transloc_route_id: 1,
        });
        assert!(
            infer_stop_observation(&history, &gtfs.trips["next"].stop_times[1], 33120, None)
                .is_none()
        );
    }
}
