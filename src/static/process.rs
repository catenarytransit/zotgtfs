use csv::Writer;
use geo_types::geometry::LineString;
use gtfs_structures;
use polyline;
use reqwest::{header, Client as ReqwestClient};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::fs::File;
use std::io::Write;

#[derive(Deserialize, Debug, Serialize)]
struct ScheduleManualInput {
    name: String,
    routeorder: Vec<String>,
    timed: Vec<String>,
    files: Vec<ScheduleFiles>,
}

#[derive(Deserialize, Debug, Serialize)]
struct ScheduleFiles {
    name: String,
    //true if monday-thursday, false if friday only
    monthurs: bool,
}

#[derive(Deserialize, Debug, Serialize)]
struct TranslocAgencies {
    rate_limit: f32,
    expires_in: f32,
    api_latest_version: String,
    generated_on: String,
    data: Vec<TranslocAgency>,
    api_version: String,
}

#[derive(Deserialize, Debug, Serialize, Clone)]
struct TranslocPos {
    lat: f32,
    lng: f32,
}

#[derive(Deserialize, Debug, Serialize)]
struct TranslocAgency {
    long_name: String,
    language: String,
    position: TranslocPos,
    short_name: String,
    name: String,
    phone: Option<String>,
    url: String,
    timezone: String,
    bounding_box: Vec<TranslocPos>,
    agency_id: String,
}

#[derive(Deserialize, Debug, Serialize)]
struct TranslocSegments {
    data: BTreeMap<String, String>,
    api_version: String,
    rate_limit: f32,
    expires_in: f32,
    api_latest_version: String,
    generated_on: String,
}

#[derive(Deserialize, Debug, Serialize)]
struct TranslocRoute {
    description: String,
    long_name: String,
    segments: Vec<Vec<String>>,
    short_name: String,
    //does not have #
    color: String,
    text_color: String,
    is_active: bool,
    route_id: String,
    agency_id: i32,
    url: String,
    #[serde(rename(deserialize = "type"))]
    route_type: String,
    is_hidden: bool,
    stops: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct TranslocRoutes {
    data: BTreeMap<String, Vec<TranslocRoute>>,
    api_version: String,
    rate_limit: f32,
    expires_in: f32,
    api_latest_version: String,
    generated_on: String,
}

#[derive(Deserialize, Debug, Clone)]
struct TranslocStop {
    code: String,
    description: String,
    url: String,
    parent_station_id: Option<String>,
    agency_ids: Vec<String>,
    station_id: Option<String>,
    location_type: String,
    location: TranslocPos,
    stop_id: String,
    routes: Vec<String>,
    name: String,
}

#[derive(Deserialize, Debug)]
struct TranslocStops {
    data: Vec<TranslocStop>,
    api_version: String,
    rate_limit: f32,
    expires_in: f32,
    api_latest_version: String,
    generated_on: String,
}

fn hextorgb(x: String) -> rgb::RGB<u8> {
    let numberarray: Vec<char> = x.chars().collect();

    let mut rgbarray = vec![];

    for i in 0..3 {
        let mut hex = numberarray[i * 2].to_string();
        hex.push_str(numberarray[i * 2 + 1].to_string().as_str());

        rgbarray.push(u8::from_str_radix(&hex, 16).unwrap());
    }

    return rgb::RGB8 {
        r: rgbarray[0],
        g: rgbarray[1],
        b: rgbarray[2],
    };
}

#[tokio::main]
async fn main() {
    let manualschedule: Vec<ScheduleManualInput> =
        serde_json::from_str(fs::read_to_string("route-sup.json").unwrap().as_str()).unwrap();

    println!("{:?}", &manualschedule);

    let manualhashmap: HashMap<String, ScheduleManualInput> =
        HashMap::from_iter(manualschedule.into_iter().map(|x| (x.name.clone(), x)));

    let agenciesjson =
        fs::read_to_string("staticfiles/agencies.json").expect("Unable to read file");
    let agencies: TranslocAgencies = serde_json::from_str(&agenciesjson).unwrap();

    let gtfs_agencies = agencies
        .data
        .iter()
        .map(|agency| gtfs_structures::Agency {
            id: Some(agency.agency_id.clone()),
            name: agency.name.clone(),
            url: agency.url.clone(),
            timezone: String::from("America/Los_Angeles"),
            lang: Some(agency.language.clone()),
            phone: agency.phone.clone(),
            fare_url: None,
            email: None,
        })
        .collect::<Vec<gtfs_structures::Agency>>();

    println!("gtfs_agencies: {:?}", gtfs_agencies);

    let mut wtr = Writer::from_writer(vec![]);
    for agency in gtfs_agencies {
        wtr.serialize(agency).unwrap();
    }

    let gtfs_agencies_csv = String::from_utf8(wtr.into_inner().unwrap()).unwrap();

    println!("gtfs_agencies_csv: {:?}", gtfs_agencies_csv);

    let mut file = File::create("anteater_gtfs/agencies.txt").unwrap();
    file.write_all(gtfs_agencies_csv.as_bytes()).unwrap();

    //time to decompile the segments :-(

    let segmentsjson =
        fs::read_to_string("staticfiles/segments.json").expect("Unable to read file");

    let segments: TranslocSegments = serde_json::from_str(&segmentsjson).unwrap();

    let routesjson = fs::read_to_string("staticfiles/routes.json").expect("Unable to read file");

    let routes: TranslocRoutes = serde_json::from_str(&routesjson).unwrap();

    let stopsjson = fs::read_to_string("staticfiles/stops.json").expect("Unable to read file");

    let stops: TranslocStops = serde_json::from_str(&stopsjson).unwrap();

    let mut segments_map: HashMap<String, LineString<f64>> = HashMap::new();

    let stopcode_to_stopid: HashMap<String, String> = HashMap::from_iter(
        stops
            .data
            .clone()
            .into_iter()
            .map(|stop: TranslocStop| (stop.code.clone(), stop.stop_id.clone())),
    );

    let mut stopswriter = Writer::from_writer(vec![]);

    let stopshashmap: HashMap<String, gtfs_structures::Stop> =
        HashMap::from_iter(stops.data.clone().into_iter().map(|stop: TranslocStop| {
            (
                stop.stop_id.clone(),
                gtfs_structures::Stop {
                    id: stop.stop_id.clone(),
                    code: Some(stop.code.clone()),
                    name: stop.name.clone(),
                    description: stop.description.clone(),
                    location_type: gtfs_structures::LocationType::StopPoint,
                    parent_station: None,
                    zone_id: None,
                    url: None,
                    timezone: Some(String::from("America/Los_Angeles")),
                    latitude: Some(stop.location.lat.into()),
                    longitude: Some(stop.location.lng.into()),
                    wheelchair_boarding: gtfs_structures::Availability::Available,
                    level_id: None,
                    platform_code: None,
                    transfers: vec![],
                    pathways: vec![],
                },
            )
        }));

    //STOPS
    for stop in stops.data.iter() {
        stopswriter
            .serialize(gtfs_structures::Stop {
                id: stop.stop_id.clone(),
                code: Some(stop.code.clone()),
                name: stop.name.clone(),
                description: stop.description.clone(),
                location_type: gtfs_structures::LocationType::StopPoint,
                parent_station: None,
                zone_id: None,
                url: None,
                timezone: Some(String::from("America/Los_Angeles")),
                latitude: Some(stop.location.lat.into()),
                longitude: Some(stop.location.lng.into()),
                wheelchair_boarding: gtfs_structures::Availability::Available,
                level_id: None,
                platform_code: None,
                transfers: vec![],
                pathways: vec![],
            })
            .unwrap();
    }

    let stops_csv = String::from_utf8(stopswriter.into_inner().unwrap()).unwrap();
    let mut stopsfile = File::create("anteater_gtfs/stops.txt").unwrap();

    stopsfile.write_all(stops_csv.as_bytes()).unwrap();

    for (segment_id, segment_data) in segments.data.iter() {
        //println!("{}, {}",segment_id, segment_data)

        let segment_polyline = polyline::decode_polyline(segment_data, 5).unwrap();

        segments_map.insert(segment_id.clone(), segment_polyline);
    }

    println!("segments_map: {:?}", segments_map);

    let mut shapeswriter = Writer::from_writer(vec![]);

    let mut routeswriter = Writer::from_writer(vec![]);

    for (agency_id, routes_array) in routes.data.iter() {
        for route in routes_array {
            // println!("Route: {:?}", route);

            let mut vec_of_trips: Vec<gtfs_structures::RawTrip> = vec![];
            let mut vec_of_stop_times: Vec<gtfs_structures::RawStopTime> = vec![];

            routeswriter
                .serialize(gtfs_structures::Route {
                    id: route.route_id.clone(),
                    agency_id: Some(String::from(agency_id)),
                    short_name: route.short_name.clone(),
                    long_name: route.long_name.clone(),
                    desc: Some(route.description.clone()),
                    route_type: gtfs_structures::RouteType::Bus,
                    url: Some(format!(
                        "https://shuttle.uci.edu/routes/{}-line",
                        route.short_name.clone().as_str().to_lowercase()
                    )),
                    color: hextorgb(route.color.clone()),
                    text_color: hextorgb(route.text_color.clone()),
                    order: None,
                    continuous_pickup: gtfs_structures::ContinuousPickupDropOff::NotAvailable,
                    continuous_drop_off: gtfs_structures::ContinuousPickupDropOff::NotAvailable,
                })
                .unwrap();

            let mut bigstackofpoints: LineString<f64> = LineString(vec![]);

            for segment_part in &route.segments {
                let segment_id = segment_part[0].clone();
                //can be "forward" or "backward"
                let segment_direction = segment_part[1].clone();

                let mut segment = segments_map.get(&segment_id).unwrap().clone().into_inner();

                if segment_direction == "backward" {
                    segment.reverse();
                }

                bigstackofpoints = bigstackofpoints
                    .into_iter()
                    .chain(segment.into_iter())
                    .collect::<LineString<f64>>();
            }

            //now they have to be seperated and put into the shapes list

            let this_routes_shape_id = format!("{}shape", route.route_id);

            let mut seqcount = 0;

            for point in bigstackofpoints.into_iter() {
                shapeswriter
                    .serialize(gtfs_structures::Shape {
                        id: this_routes_shape_id.clone(),
                        latitude: point.y,
                        longitude: point.x,
                        sequence: seqcount,
                        dist_traveled: None,
                    })
                    .unwrap();

                seqcount = seqcount + 1;
            }

            //make the schedule here

            let data_to_use = manualhashmap.get(&route.short_name).unwrap();

            println!("data_to_use: {:?}", data_to_use);

            for file in data_to_use.files.iter() {
                // let scheduletimes = fs::read_to_string(format!("schedules/{}", file.name).as_str()).unwrap().as_str();

                let mut rdr = csv::Reader::from_path(format!("schedules/{}", file.name)).unwrap();
                let vecofrows = rdr
                    .records()
                    .map(|x| x.unwrap())
                    .collect::<Vec<csv::StringRecord>>();

                println!("vecofrows: {:?}", vecofrows);

                let headers = rdr.headers().unwrap();

                if (headers.get(0).unwrap() == "repeat_interval"
                    && headers.get(1).unwrap() == "repeat_number"
                    && headers.len() >= 3)
                {
                    //convert header to vec
                    let mut headervec = vec![];

                    for i in 2..headers.len() {
                        headervec.push(headers.get(i).unwrap().to_string());
                    }

                    struct Stoptimepre {
                        stop_id: String,
                        stop_code: String,
                        timed: bool,
                        arrivals: Option<u32>,
                        departures: Option<u32>,
                        enabled: bool,
                    }

                    let mut tripnumber = 1;

                    //for each row
                    for row in vecofrows {
                        let mut rowvec: Vec<Stoptimepre> = vec![];

                        let mut scheduleforthistime: Vec<String> = vec![];

                        for i in 2..row.len() {
                            scheduleforthistime.push(row.get(i).unwrap().to_string());
                        }

                        //split into array of f32 based on /, minutes to seconds
                        let repeatinterval = row
                            .get(0)
                            .unwrap_or_else(|| "0")
                            .to_string()
                            .split("/")
                            .collect::<Vec<&str>>()
                            .iter()
                            .map(|x| x.parse::<f32>().unwrap_or_else(|x| 0.0) * 60.0)
                            .collect::<Vec<f32>>();

                        println!("next bus in {:?}s", repeatinterval);

                        let repeatnumberoftimes = row
                            .get(1)
                            .unwrap_or_else(|| "0")
                            .to_string()
                            .parse::<u32>()
                            .unwrap_or_else(|x| 1);

                        println!("repeat {:?} times", repeatnumberoftimes);

                      
                          /*
                            rowvec.push(Stoptimepre {
                                stop_id: stopcode_to_stopid
                                    .get(&routelooppoint.clone())
                                    .unwrap()
                                    .clone(),
                                stop_code: routelooppoint.clone(),
                                timed: data_to_use.timed.contains(&routelooppoint.clone()),
                                arrivals: None,
                                departures: None,
                                enabled: true,
                            }); */
                            for routelooppoint in data_to_use.routeorder.iter() {
                        }

                        tripnumber = tripnumber + 1;
                    }
                }
            }
        }
    }

    let shapes_csv = String::from_utf8(shapeswriter.into_inner().unwrap()).unwrap();
    let mut shapesfile = File::create("anteater_gtfs/shapes.txt").unwrap();

    shapesfile.write_all(shapes_csv.as_bytes()).unwrap();

    let routes_csv = String::from_utf8(routeswriter.into_inner().unwrap()).unwrap();
    let mut routesfile = File::create("anteater_gtfs/routes.txt").unwrap();

    routesfile.write_all(routes_csv.as_bytes()).unwrap();
}
