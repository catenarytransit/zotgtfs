use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::fs;
use std::io::Write;
use csv::Writer;
use polyline;
use gtfs_structures;
use geo_types::geometry::LineString;

#[derive(Deserialize, Debug, Serialize)]
struct TranslocAgencies {
    rate_limit: f32,
    expires_in: f32,
    api_latest_version: String,
    generated_on: String,
    data: Vec<TranslocAgency>,
    api_version: String,
}

#[derive(Deserialize, Debug, Serialize)]
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

#[derive(Deserialize, Debug)]
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

fn hextorgb(x:String) -> rgb::RGB<u8> {
    let numberarray: Vec<char> = x.chars().collect();

    let mut rgbarray = vec![];

    for i in 0..3 {
        let mut hex = numberarray[i*2].to_string();
        hex.push_str(numberarray[i*2+1].to_string().as_str());

        rgbarray.push(u8::from_str_radix(&hex, 16).unwrap());
    }

    return rgb::RGB8 {r: rgbarray[0], g:rgbarray[1], b:rgbarray[2]};
}

#[tokio::main]
async fn main() {
    let agenciesjson = fs::read_to_string("staticfiles/agencies.json").expect("Unable to read file");
    let agencies:TranslocAgencies = serde_json::from_str(&agenciesjson).unwrap();

    let gtfs_agencies = agencies.data.iter().map(|agency| {
        gtfs_structures::Agency {
            id: Some(agency.agency_id.clone()),
            name: agency.name.clone(),
            url: agency.url.clone(),
            timezone: String::from("America/Los_Angeles"),
            lang: Some(agency.language.clone()),
            phone: agency.phone.clone(),
            fare_url: None,
            email: None,
        }
    }).collect::<Vec<gtfs_structures::Agency>>();

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

    let segmentsjson = fs::read_to_string("staticfiles/segments.json").expect("Unable to read file");

    let segments:TranslocSegments = serde_json::from_str(&segmentsjson).unwrap();

    let  routesjson = fs::read_to_string("staticfiles/routes.json").expect("Unable to read file");

    let routes:TranslocRoutes = serde_json::from_str(&routesjson).unwrap();

    let stopsjson = fs::read_to_string("staticfiles/stops.json").expect("Unable to read file");

    let stops:TranslocStops = serde_json::from_str(&stopsjson).unwrap();

    let mut segments_map:HashMap<String, LineString<f64>> = HashMap::new();

        let mut stopswriter = Writer::from_writer(vec![]);

        //STOPS
        for stop in stops.data.iter() {
            stopswriter.serialize(gtfs_structures::Stop {
                id: stop.stop_id.clone(),
                code: Some(stop.code.clone()),
                name: stop.name.clone(),
                description: stop.description.clone(),
                location_type: gtfs_structures::LocationType::StopPoint,
                parent_station: None,
                zone_id: None,
                url: Some(stop.url.clone()),
                timezone: Some(String::from("America/Los_Angeles")),
                latitude: Some(stop.location.lat.into()),
                longitude: Some(stop.location.lng.into()),
                wheelchair_boarding: gtfs_structures::Availability::Available,
                level_id: None,
                platform_code: None,
                transfers: vec![],
                pathways: vec![]
            }).unwrap();
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
            println!("Route: {:?}", route);

            routeswriter.serialize(gtfs_structures::Route {
                id: route.route_id.clone(),
                agency_id: Some(String::from(agency_id)),
                short_name: route.short_name.clone(),
                long_name: route.long_name.clone(),
                desc: Some(route.description.clone()),
                route_type: gtfs_structures::RouteType::Bus,
                url: Some(route.url.clone()),
                color: hextorgb(route.color.clone()),
                text_color: hextorgb(route.text_color.clone()),
                order: None,
                continuous_pickup: gtfs_structures::ContinuousPickupDropOff::NotAvailable,
                continuous_drop_off: gtfs_structures::ContinuousPickupDropOff::NotAvailable,
            }).unwrap();

            let mut bigstackofpoints:LineString<f64> = LineString(vec![]);

        for segment_part in &route.segments {
            let segment_id = segment_part[0].clone();
            //can be "forward" or "backward"
            let segment_direction = segment_part[1].clone();

            let mut segment = segments_map.get(&segment_id).unwrap().clone().into_inner();

            if segment_direction == "backward" {
                segment.reverse();
            }

            bigstackofpoints = bigstackofpoints.into_iter().chain(segment.into_iter()).collect::<LineString<f64>>();
        }

        //now they have to be seperated and put into the shapes list

        let this_routes_shape_id = format!("{}shape",route.route_id);

        let mut seqcount = 0;

        for point in bigstackofpoints.into_iter() {
            shapeswriter.serialize(gtfs_structures::Shape {
                id: this_routes_shape_id.clone(),
                latitude: point.y,
                longitude: point.x,
                sequence: seqcount,
                dist_traveled: None,
            }).unwrap();

            seqcount = seqcount + 1;
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
