# For devs

Hi Catenary contributors!

Anteater express has done it again and screwed us over! looks like they have an entirely new website now.
https://shuttle.uci.edu/ Unfortunately, I am too busy to deal with this!

Your goal is to provide a function which returns Gtfs-rt. You can return the vehicles / trips seperately in the same struct, or put them in the same protobuf data structure. Up to you. 

Here's an example function signature:

```rs
pub async fn get_gtfs_rt() -> Result<gtfs_rt::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    // your code here
}
```

Or 

```rs
pub struct AntExGtfsRt {
    pub vehicle_positions: gtfs_rt::FeedMessage,
    pub trip_positions: gtfs_rt::FeedMessage,
}
pub async fn get_gtfs_rt() -> Result<AntExGtfsRt, Box<dyn std::error::Error + Send + Sync>> {
    // your code here
}
```

The code should be in `src/lib.rs`, please make some test functions using rust's built in test features! Please also write comments in your code to explain your code or help other students read it more easily.

Documentation for GTFS-rt can be found here:
https://docs.rs/gtfs-rt/latest/gtfs_rt/
and here https://gtfs.org/

The rust crate for schedule gtfs is here: https://docs.rs/gtfs-structures

Good luck reverse engineering! You may work in groups and compare answers with other students.
- Kyler