use gtfs_rt::vehicle_position::*;
use gtfs_rt::*;
use serde::Deserialize;
use serde_json::from_str;
use std::error::Error;
use std::time::SystemTime;
// Author(s): Jacob Whitecotton
// Version: 4/6/2024

/**
 * Fetches jsonp data from ucirvine's transit feed and converts it into gtfs_rt
 */
pub async fn get_gtfs_rt() -> Result<gtfs_rt::FeedMessage, Box<dyn std::error::Error + Send + Sync>>
{
    let data = reqwest::get("https://ucirvine.transloc.com/Services/JSONPRelay.svc/GetMapVehiclePoints?method=jQuery111104379215856036027_1712182850874&ApiKey=8882812681&_=1712182850877")
        .await?
        .text()
        .await?;
    gtfs_rt_from_string(data)
}

/**
 * Function creates gtfs from a string, called by get_gtfs_rt and used for testing.
 */
fn gtfs_rt_from_string(
    data: String,
) -> Result<gtfs_rt::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    let data = parse_data(data)?;
    // if data parsed is empty (at night for example) returns an empty gtfs_rt feed.
    if data.len() == 0 {
        let empty_entity: Vec<gtfs_rt::FeedEntity> = Vec::new();
        let empty_data = gtfs_rt::FeedMessage {
            header: FeedHeader {
                gtfs_realtime_version: String::from("2.0"),
                incrementality: None,
                timestamp: None,
            },
            entity: empty_entity,
        };
        return Ok(empty_data);
    }
    let mut anteater_entities: Vec<FeedEntity> = Vec::new();
    for i in 0..data.len() {
        let vehicle = match data.get(i) {
            Some(x) => x,
            None => {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Invalid String",
                )))
            }
        };
        anteater_entities.push(FeedEntity {
            id: i.to_string(),
            is_deleted: Some(false),
            trip_update: None,
            vehicle: Some(vehicle.get_vehicle_position()),
            alert: None,
            shape: None,
            stop: None,
            trip_modifications: None
        });
    }
    let anteater_gtfs = gtfs_rt::FeedMessage {
        header: FeedHeader {
            gtfs_realtime_version: String::from("2.0"),
            incrementality: Some(1),
            timestamp: Some(SystemTime::now().elapsed()?.as_secs()),
        },
        entity: anteater_entities.to_owned(),
    };
    return Ok(anteater_gtfs);
}

/**
 * Struct represents one vehicle in the jsonp provided by ucirvine.
 */
#[derive(Deserialize)]
struct AnteaterExpressData {
    #[serde(rename = "GroundSpeed")]
    ground_speed: f32,
    #[serde(rename = "Heading")]
    heading: f32,
    #[serde(rename = "Latitude")]
    latitude: f32,
    #[serde(rename = "Longitude")]
    longitude: f32,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "RouteID")]
    route_id: i16,
    #[serde(rename = "VehicleID")]
    vehicle_id: i16,
}

/**
 * Implementation block of AnteaterExpressData. Functions
 * generate pieces of gtfs_rt data to be used in
 * gtfs_rt_from_string.
 */
impl AnteaterExpressData {
    /**
     * Generates gtfs_rt::CarriageDetails based on self's data.
     * gtfs_data assumes all vehicles are non-articulated (as
     * of now they are not/no carriage details are provided).
     */
    fn get_carriage_details(&self) -> CarriageDetails {
        CarriageDetails {
            id: Some(self.vehicle_id.clone().to_string()),
            label: Some(self.vehicle_id.clone().to_string()),
            occupancy_status: None,
            occupancy_percentage: None,
            carriage_sequence: Some(1),
        }
    }

    /**
     * Generates a gtfs_rt::Position based on self's data.
     */
    fn get_position(&self) -> Position {
        Position {
            latitude: self.latitude,
            longitude: self.longitude,
            bearing: Some(self.heading),
            odometer: None,
            speed: Some((self.ground_speed as f32 * (1.0 / 3.6)) as f32),
        }
    }

    /**
     * Generates a gtfs_rt::TripDescriptor based on self's data.
     */
    fn get_trip_descriptor(&self) -> TripDescriptor {
        TripDescriptor {
            trip_id: Some(String::from("")),
            route_id: Some(self.route_id.clone().to_string()),
            direction_id: Some(0),
            start_time: None,
            start_date: None,
            schedule_relationship: None,
            modified_trip: None,
        }
    }

    /**
     * Generates a gtfs_rt::VehicleDescriptor based on self's
     * data.
     */
    fn get_vehicle_descriptor(&self) -> VehicleDescriptor {
        VehicleDescriptor {
            id: Some(self.vehicle_id.to_string().clone()),
            label: Some(self.name.clone()),
            license_plate: None,
            wheelchair_accessible: None,
        }
    }

    /**
     * Generates a gtfs_rt::VehiclePosition based on self's
     * data and the impl functions above.
     */
    fn get_vehicle_position(&self) -> VehiclePosition {
        VehiclePosition {
            trip: Some(self.get_trip_descriptor()),
            vehicle: Some(self.get_vehicle_descriptor()),
            position: Some(self.get_position()),
            current_stop_sequence: None, //fetch from gtfs
            stop_id: None,               //fetch from gtfs
            current_status: None,
            timestamp: None,
            congestion_level: None,
            occupancy_status: None,
            occupancy_percentage: None,
            multi_carriage_details: vec![self.get_carriage_details()], //Length of 1 since anteater express does not provide carriage details
        }
    }
}

/**
 * Parses the AnteaterExpress feed, in the form of a string,
 * into a struct, or returns an error if the string provided
 * is invalid.
 */
fn parse_data(data: String) -> Result<Vec<AnteaterExpressData>, Box<dyn Error + Send + Sync>> {
    let prefix_index = data.find("(");
    let suffix_index = data.chars().rev().position(|x| x == ')');
    if let (Some(prefix_index), Some(suffix_index)) = (prefix_index, suffix_index) {
        //index at which data starts
        let suffix_index = data.len() - (suffix_index + 1);
        //index at which data ends
        let prefix_index = prefix_index + 1;

        if suffix_index - prefix_index < 100 {
            let nothing: Vec<AnteaterExpressData> = Vec::new();
            return Ok(nothing);
        }
        // data is iterated through, skips up to prefix_index, takes up to suffix_index, and collects the values

        let data: String = data
            .chars()
            .skip(prefix_index)
            .take(suffix_index - prefix_index)
            .collect();
        let data: Vec<AnteaterExpressData> = from_str(&data)?;
        Ok(data)
    } else {
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Invalid String",
        )))
    }
}

/**
 * Test block
 */
#[cfg(test)]
mod tests {
    use super::*;

    fn create_example_string() -> String {
        String::from(
            r#"jQuery111104379215856036027_1712182850874( 
                [
                    {
                        "GroundSpeed":10.99901573793,
                        "Heading":78,
                        "IsDelayed":false,
                        "IsOnRoute":true,
                        "Latitude":33.64704,
                        "Longitude":-117.82938,
                        "Name":"AE02",
                        "RouteID":4,
                        "Seconds":3,
                        "TimeStamp":"\/Date(1712229203000-0600)\/",
                        "VehicleID":3
                    },
                    {
                        "GroundSpeed":18.79921258116,
                        "Heading":289,
                        "IsDelayed":false,
                        "IsOnRoute":true,
                        "Latitude":33.6478503,
                        "Longitude":-117.8260522,
                        "Name":"AE04",
                        "RouteID":4,
                        "Seconds":2,
                        "TimeStamp":"\/Date(1712229204000-0600)\/",
                        "VehicleID":5
                    },
                    {
                        "GroundSpeed":0.10066213305,
                        "Heading":0,
                        "IsDelayed":false,
                        "IsOnRoute":true,
                        "Latitude":33.6489682,
                        "Longitude":-117.839556,
                        "Name":"AE11",
                        "RouteID":4,
                        "Seconds":2,
                        "TimeStamp":"\/Date(1712229204000-0600)\/",
                        "VehicleID":10
                    },
                    {
                        "GroundSpeed":0,
                        "Heading":0,
                        "IsDelayed":false,
                        "IsOnRoute":true,
                        "Latitude":33.6489278979,
                        "Longitude":-117.8448493721,
                        "Name":"AE08",
                        "RouteID":5,
                        "Seconds":2,
                        "TimeStamp":"\/Date(1712229204000-0600)\/",
                        "VehicleID":7
                    }
                ] 
            );"#,
        )
    }

    fn create_no_data_string() -> String {
        String::from(
            "jQuery111104379215856036027_1712182850874(
            [
                {
                }
            ] 
        )",
        )
    }

    #[test]
    fn parse_data_is_ok() {
        let example_data = create_example_string();
        let deserialized_data = parse_data(example_data);
        assert!(deserialized_data.is_ok());
    }

    #[test]
    fn parse_data_is_correct() {
        let example_data = create_example_string();
        let deserialized_data = parse_data(example_data).unwrap();
        let vehicle_0 = deserialized_data.get(0).unwrap();
        let vehicle_2 = deserialized_data.get(2).unwrap();

        assert_eq!(vehicle_0.ground_speed, 10.99901573793);
        assert_eq!(vehicle_2.vehicle_id, 10);
    }

    #[test]
    fn parse_data_no_data() {
        let example_data = create_no_data_string();
        let deserialized_data = parse_data(example_data);
        assert!(deserialized_data.is_ok());
    }

    #[test]
    fn gtfs_rt_from_string_no_data() {
        let example_data = create_no_data_string();
        let anteater_gtfs = gtfs_rt_from_string(example_data);
        assert!(anteater_gtfs.is_ok());
    }

    #[test]
    fn gtfs_rt_from_string_is_ok() {
        let example_data = create_example_string();
        let anteater_gtfs = gtfs_rt_from_string(example_data);
        assert!(anteater_gtfs.is_ok());
    }

    #[test]
    fn gtfs_rt_from_string_same_length_as_express_data() {
        let example_data = create_example_string();
        let anteater_data = parse_data(example_data).unwrap();
        let example_data = create_example_string();
        let anteater_gtfs = gtfs_rt_from_string(example_data).unwrap();
        assert_eq!(anteater_data.len(), anteater_gtfs.entity.len());
    }

    #[test]
    fn gtfs_rt_from_string_is_correct() {
        let example_data = create_example_string();
        let anteater_data = gtfs_rt_from_string(example_data).unwrap();
        let entity_1 = anteater_data.entity.get(1).unwrap();
        let expected_heading: f32 = 289.0;
        let entity_3 = anteater_data.entity.get(3).unwrap();
        let entity_0 = anteater_data.entity.get(0).unwrap();
        assert_eq!(
            entity_1
                .clone()
                .vehicle
                .unwrap()
                .position
                .unwrap()
                .bearing
                .unwrap(),
            expected_heading
        );
        assert_eq!(
            entity_3
                .clone()
                .vehicle
                .unwrap()
                .vehicle
                .unwrap()
                .id
                .unwrap(),
            String::from("7")
        );
        assert_eq!(
            entity_0
                .clone()
                .vehicle
                .unwrap()
                .trip
                .unwrap()
                .route_id
                .unwrap(),
            String::from("4")
        );
    }

    #[tokio::test]
    async fn get_gtfs_rt_is_ok() {
        assert!(get_gtfs_rt().await.is_ok());
    }

    // This only passes when Anteater Express is in service (lol)
    #[tokio::test]
    async fn get_gtfs_rt_is_correct() {
        let anteater_data = get_gtfs_rt().await.unwrap();
        let entity_0 = anteater_data.entity.get(0).unwrap();
        assert!(entity_0
            .clone()
            .vehicle
            .unwrap()
            .position
            .unwrap()
            .bearing
            .is_some());
        assert!(entity_0
            .clone()
            .vehicle
            .unwrap()
            .vehicle
            .unwrap()
            .id
            .is_some());
        assert!(entity_0
            .clone()
            .vehicle
            .unwrap()
            .trip
            .unwrap()
            .route_id
            .is_some());

        println!("{:?}", anteater_data);
    }
}
