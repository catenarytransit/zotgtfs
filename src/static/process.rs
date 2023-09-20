use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Write;

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
    languge: String,
    position: TranslocPos,
    short_name: String,
    name: String,
    phone: Option<String>,
    url: String,
    timezone: String,
    boundingbox: Vec<TranslocPos>,
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
    segments: Vec<String>,
    short_name: String,
    //does not have #
    color: String,
    text_color: String,
    is_active: bool,
    route_id: String,
    agency_id: String,
    url: String,
    #[serde(rename(deserialize = "type"))]
    route_type: String,
    is_hidden: bool,
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

#[tokio::main]
async fn main() {
    let agenciesjson = fs::read_to_string("staticfiles/agencies.json").expect("Unable to read file");
    let agencies = serde_json::from_str::<TranslocAgencies>(&agenciesjson).unwrap();
}
