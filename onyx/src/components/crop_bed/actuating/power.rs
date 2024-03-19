use crate::devices::hardware::pdm::{Pdm, PdmConfig};
use crate::messages::control::weed::WeedMessage;
use chrono::{DateTime, Duration, Utc};
use priority_queue::DoublePriorityQueue;
use serde::{Deserialize, Serialize};
use socketcan::tokio::CanSocket as AsyncCanSocket;
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::Instant,
};
use uuid::Uuid;

/// Spray bound is a time constant in microseconds that determines
/// if a spray message is close enough to the current UTC time to
/// be sent to the PDM to be sprayed. There is some fluctuation
/// here because the tokio based thread sleep may introduce some
/// drift, although this has not been seen in tests.
// TODO: move this to yaml config.
const SPRAY_BOUND: i64 = 5;

/// Set the configuration for a crop bed power component.
/// This is created by grouping multiple PDMs with different
/// addresses on a canbus trunk line which are wired to
/// actuated solenoids.
#[derive(Deserialize, Serialize, PartialEq, Eq, Debug)]
pub struct CropBedPowerConfig {
    /// ID of the crop be the component is attached to.
    crop_bed_id: u8,
    /// The addressable canbus interface ID.
    canbus_id: String,
    /// The internal linux socket that the component listens to for incoming messages.
    port: i32,
    /// Map of the config files used to generate the PDMs, as per the technical specification.
    pdm_config_files: HashMap<u8, PathBuf>,
    /// Due to the way electrical wanted to wire the harnesses channel
    /// numbers do not always match with the expected solenoid actuator.
    /// This map translates these wiring IDs.
    // NOTE: Remember this when implementing logging and telemetry as it
    // will likely lead to confusion.
    channel_map: Option<HashMap<u8, (u8, u8)>>,
}

/// Convert received weed messages into a type that suits a
/// priority queue. The original weed message sends information
/// about starting and stopping the weed message, where as the
/// queue saves messages for both on and off.
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct WeedQueueMessage {
    /// Channels to actuate.
    pub channels: Vec<u8>,
    /// UTC time when the action should take place.
    pub time_to_fire: DateTime<Utc>,
    /// Power is turned on, i.e. PWM 100.
    pub is_on: bool,
    /// Spray starts is used in  loop to prune overlapping messages.
    pub original_spray_starts: DateTime<Utc>,
    /// Spray ending is used in  loop to prune overlapping messages.
    pub original_spray_ending: DateTime<Utc>,
}

impl CropBedPowerConfig {
    /// Crop bed power configuration.
    ///
    /// * `crop_bed_id`: module ids from [0 - 2]
    /// * `canbus_id`: String for the bus i.e., can0.
    pub fn new(
        crop_bed_id: u8,
        canbus_id: String,
        port: i32,
        channel_map: Option<HashMap<u8, (u8, u8)>>,
    ) -> Self {
        Self {
            port,
            crop_bed_id,
            canbus_id,
            pdm_config_files: HashMap::new(),
            channel_map,
        }
    }

    /// Add a PDM to the component with a config file.
    ///
    /// * `filepath`: path to config
    /// * `pdm_id`: Address of the PDM.
    pub fn add_pdm_config_file<F>(mut self, filepath: F, pdm_id: u8) -> Self
    where
        F: AsRef<OsStr>,
    {
        self.pdm_config_files.insert(pdm_id, (&filepath).into());
        self
    }

    /// Create a new `PdmConfig` by reading parameters stored in a file.
    ///
    /// * `filepath`: filepath to the stored parameters.
    pub fn from_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let file = Path::new(&filepath);
        if file.is_file() {
            let config_file = config::Config::builder()
                .add_source(config::File::new(
                    &file.to_string_lossy(),
                    config::FileFormat::Yaml,
                ))
                .build()
                .expect("Failed read config");

            config_file
                .try_deserialize::<CropBedPowerConfig>()
                .expect("Failed to parse config file into struct")
        } else {
            panic!("Could not locate the config file {:?}", file);
        }
    }
}

/// Component for managing the crop bed power in one module.
/// Currently this consists of two PDMs, but could be increased
/// to as many as allowed on the canbus network (pending addressing
/// clashing)
#[allow(dead_code)]
pub struct CropBedPower {
    /// Unique identifier for the component.
    uuid: Uuid,
    /// ID of the specific crop bed module.
    crop_bed_id: u8,
    /// Canbus interface name.
    canbus_id: String,
    /// Map of the Pdm drivers.
    pdms: HashMap<u8, Pdm>,
    /// Internal linux port the component will be commanded on
    port: i32,
    /// Message queue that stores upcoming actions.
    message_queue: DoublePriorityQueue<WeedQueueMessage, DateTime<Utc>>,
    /// Channel maps for the PDMs when the wiring harness does
    /// not logically map to the solenoid numbers.
    channel_map: Option<HashMap<u8, (u8, u8)>>,
}

impl CropBedPower {
    /// Create a new component from a config struct.
    ///
    /// * `config`: Struct containing the parameters for configuration.
    pub fn new(config: CropBedPowerConfig) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            port: config.port,
            crop_bed_id: config.crop_bed_id,
            canbus_id: config.canbus_id.clone(),
            channel_map: config.channel_map.clone(),
            pdms: Self::build_from_config(config),
            message_queue: DoublePriorityQueue::new(),
        }
    }

    /// Create a new component by reading the config parameters from a file.
    ///
    /// * `filepath`: path to config file.
    pub fn from_config_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let config = CropBedPowerConfig::from_file(filepath);
        Self::new(config)
    }

    /// Helper function used to build the resulting component.
    ///
    /// * `config`: struct with configuration parameters.
    fn build_from_config(config: CropBedPowerConfig) -> HashMap<u8, Pdm> {
        let mut pdms = HashMap::new();
        for (bed_position, pdm_config_file) in config.pdm_config_files {
            let pdm = Pdm::new(PdmConfig::from_file(pdm_config_file));
            pdms.insert(bed_position, pdm);
        }
        pdms
    }

    /// Add weed message to queue once parsed from the AI system.
    /// NOTE: There is an edge case where a channel will send a
    /// message that may turn off another channel pre-emptively
    /// this message was stripped out in a previous implementation
    /// of this function however there still appeared to be noise.
    /// On further investigation it became clear that there was
    /// some additional messages being sent by the AI system both
    /// factually (i.e. weeds moving in the wind), and *Potentially*
    /// erroneously (array indexing) which may have been contributing
    /// to the noise or in the very least making it hard to trouble
    /// shoot. See git commit history for those naive implementations
    /// once the AI message generation has been confirmed.
    ///
    /// * `message`: message parsed from AI container.
    fn add_to_message_queue(&mut self, message: WeedQueueMessage) {
        let priority = message.time_to_fire;
        self.message_queue.push(message, priority);
    }

    /// I dislike this implementation, will need to work on the image messages being
    /// sent through to the control system, or some kind of state machine which can
    // be polled by futures. Ultimately it will change with the inclusion of a wheel
    // speed sensor anyway.
    // INFO: The PDMs actuate channels based in blocks 1-12, 13-24 and need to be
    //       split up accordingly.
    async fn process_message_queue(&mut self, mut last_fire: Instant) -> Instant {
        if let Some((message, priority)) = self.message_queue.peek_min() {
            let utc_now = Utc::now();
            // TODO: this bound could be adjusted as the thread sleep can miss by 1-2 microseconds which
            //       means that messages could be discarded by being 1 microsecond behind which for this
            //      system seems unreasonable as it is more beneficial to over spray than under spray.
            if *priority < utc_now {
                self.message_queue.pop_min();
            } else if let Some(delta_t) = (*priority - utc_now).num_microseconds() {
                // check if the delta is within SPRAY_BOUND microseconds (positive)
                if delta_t < SPRAY_BOUND {
                    // The first iteration of the messages coming from AI needed to check for this
                    // condition however the AI messages have changed several times as well as the
                    // partitioning of the channels so this section can most likely be removed. The
                    // implementation for the vector of required channels is much cleaner using the
                    // partition.
                    // TODO: Add test to confirm and then remove.
                    if message.channels.len() == 1 {
                        if message.channels[0] <= 12 {
                            if let Some(pdm) = self.pdms.get(&0) {
                                let pwm = if message.is_on { 100.0 } else { 0.0 };
                                let channels = vec![message.channels[0]];
                                pdm.driver.actuate_channels(17, channels, pwm).await;
                            }
                        } else if let Some(pdm) = self.pdms.get(&1) {
                            let pwm = if message.is_on { 100.0 } else { 0.0 };
                            let channels = vec![message.channels[0] - 12];
                            pdm.driver.actuate_channels(17, channels, pwm).await;
                        }
                    } else {
                        // TODO: remove in line 12, and move to const module, or PDM config.
                        let (pdm_0, pdm_1): (_, Vec<_>) = message
                            .channels
                            .clone()
                            .into_iter()
                            .partition(|x| (*x <= 12));
                        if !pdm_0.is_empty() {
                            if let Some(pdm) = self.pdms.get(&0) {
                                let pwm = if message.is_on { 100.0 } else { 0.0 };
                                pdm.driver.actuate_channels(17, pdm_0, pwm).await;
                            }
                        }
                        if !pdm_1.is_empty() {
                            if let Some(pdm) = self.pdms.get(&1) {
                                let pwm = if message.is_on { 100.0 } else { 0.0 };
                                let channels = pdm_1.clone().iter().map(|x| x - 12).collect();
                                pdm.driver.actuate_channels(17, channels, pwm).await;
                            }
                        }
                    }
                    // No need for heartbeat message as we just sent the above.
                    last_fire = Instant::now();
                    self.message_queue.pop_min();
                }
            }
        }

        // The PDM loss of can feature will come online when a signal has not
        // been received every second. This last fire signal helps keep the
        // PDM online by sending a heartbeat.
        if last_fire.elapsed() > tokio::time::Duration::from_millis(500) {
            // TODO: Potentially wrap a config handshake in here to ensure the
            // PDM has not drifted to another state.
            if let Some(pdm) = self.pdms.get(&0) {
                pdm.driver
                    .actuate_channels(17, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], 0.0)
                    .await;
            }
            if let Some(pdm) = self.pdms.get(&1) {
                pdm.driver
                    .actuate_channels(17, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], 0.0)
                    .await;
            }
            last_fire = Instant::now();
        }
        last_fire
    }
}

/// Unit struct for adding controlling behaviour to the crop bed power.
pub struct CropBedPowerController;

impl CropBedPowerController {
    /// Start the crop bed power component.
    ///
    /// * `crop_bed_power`: component
    pub async fn start(mut crop_bed_power: CropBedPower) {
        let interface = Arc::new(Mutex::new(
            AsyncCanSocket::open(&crop_bed_power.canbus_id)
                .expect("Failed to create canbus socket"),
        ));

        for pdm in crop_bed_power.pdms.values_mut() {
            pdm.initialise(interface.clone()).await;
        }
        // Bind on the loop back port from within the container
        let listener = TcpListener::bind(format!("0.0.0.0:{}", crop_bed_power.port))
            .await
            .expect("Failed to bind port");

        let thread_safe_crop_bed_power = Arc::new(Mutex::new(crop_bed_power));

        let power_processing = thread_safe_crop_bed_power.clone();

        // PDM message firing task.
        tokio::spawn(async move {
            let mut last_fire = Instant::now();
            loop {
                let mut gaurd = power_processing.lock().await;
                last_fire = gaurd.process_message_queue(last_fire).await;
                drop(gaurd);
            }
        });
        // Looping message parsing task.

        // TODO: Remove the continue, picked up with more strict clippy linting.
        //       very straight forward. Good first issue.
        #[allow(clippy::needless_continue)]
        loop {
            if let Ok((socket, _)) = listener.accept().await {
                let power_connection = thread_safe_crop_bed_power.clone();
                tokio::spawn(async move {
                    handle_connection(socket, power_connection).await;
                });
            } else {
                continue;
            }
        }
    }
}

/// Handle connection from the AI container when it sends a message.
///
/// * `socket`: `TcpStream`
/// * `power`: component
// NOTE: This interface was the issue that wasted ~ 2 weeks during testing, the previous
//       implementation relied on a long standing connection from another container and
//       taking messages off the wire at '\b', however the starmap from the AI system
//       created a new connection every time it sent a message, this lead to an
//       enormous amount of useless tokio tasks that would be looped and polled.
// TODO: Review starmap and connection function between two systems.
async fn handle_connection(mut socket: TcpStream, power: Arc<Mutex<CropBedPower>>) {
    let (read_stream, _) = socket.split();
    let mut read_stream = BufReader::new(read_stream);
    let mut data = Vec::new();

    data.clear();
    let _bytes_read = read_stream
        .read_until(b'\n', &mut data)
        .await
        .expect("Failed to read buffer");
    match serde_json::from_slice::<WeedMessage>(&data) {
        Ok(message) => {
            if message.start_spray_time > Utc::now() {
                let mut delta = message.end_spray_time - message.start_spray_time;

                let mut channels = Vec::new();
                let mut gaurd = power.lock().await;

                for channel in message.channels_to_open {
                    // The electrical team needed to wire the PDMs in a specific way to make
                    // it easier for physical manufacturing. This means that on some crop beds
                    // that the channel numbers do not coincide with the channel numbers of the
                    // PDM. This mapping can be very confusing to trouble shoot.
                    if let Some(ref channel_map) = gaurd.channel_map {
                        let (converted, _pdm) =
                            channel_map.get(&(channel + 1)).expect("No channel map");
                        channels.push(*converted);
                    } else {
                        channels.push(channel + 1);
                    }
                }
                // PDM will cut off after 1 second, so longer durations require to have
                // the message queue to be padded out.
                // TODO: pull out 100 to a constant in utils.
                if delta > Duration::seconds(1) {
                    let mut time_to_fire = message.start_spray_time;
                    while delta > Duration::milliseconds(100) {
                        let power_ons = WeedQueueMessage {
                            channels: channels.clone(),
                            time_to_fire: time_to_fire + Duration::milliseconds(100),
                            is_on: true,
                            original_spray_starts: message.start_spray_time,
                            original_spray_ending: message.end_spray_time,
                        };
                        gaurd.add_to_message_queue(power_ons);
                        time_to_fire += Duration::milliseconds(100);
                        delta = delta - Duration::milliseconds(100);
                    }
                    let power_off = WeedQueueMessage {
                        channels: channels.clone(),
                        time_to_fire: message.end_spray_time,
                        is_on: false,
                        original_spray_starts: message.start_spray_time,
                        original_spray_ending: message.end_spray_time,
                    };
                    gaurd.add_to_message_queue(power_off);
                } else {
                    let power_ons = WeedQueueMessage {
                        channels: channels.clone(),
                        time_to_fire: message.start_spray_time,
                        is_on: true,
                        original_spray_starts: message.start_spray_time,
                        original_spray_ending: message.end_spray_time,
                    };

                    let power_off = WeedQueueMessage {
                        channels,
                        time_to_fire: message.end_spray_time,
                        is_on: false,
                        original_spray_starts: message.start_spray_time,
                        original_spray_ending: message.end_spray_time,
                    };
                    gaurd.add_to_message_queue(power_ons);
                    gaurd.add_to_message_queue(power_off);
                }
                // Make sure to drop the guard strait after using in the loop.
                drop(gaurd);
            } else {
                println!("Message Ignored, recieved to late from analysis system");
            }
        }
        Err(e) => {
            println!("Received a malformed request {:?}, data: {:?}", e, &data);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serial_test::serial;
    use std::fs::OpenOptions;

    #[rstest]
    /// Test partitioning functions.
    fn test_vec_split_power() {
        let channels = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        ];
        let (a, b): (_, Vec<_>) = channels.into_iter().partition(|x| (*x <= 12));
        assert_eq!(a, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!(b, vec![13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24]);
    }

    #[rstest]
    #[serial]
    #[rustfmt::skip]
    #[case((vec![
            (1,  (11, 0)),
            (2,  (12, 0)),
            (3,  (13,  1)),
            (4,  (14,  1)),
            (5,  (15,  1)),
            (6,  (16,  1)),
            (7,  (17,  1)),
            (8,  (18,  1)),
            (9,  (19,  1)),
            (10, (20,  1)),
            (11, (21,  1)),
            (12, (22, 1)),
            (13, (23, 1)),
            (14, (24, 1)),
        ], 1, "can1", 17651))]
    #[case((vec![
            (1,  (24, 1)),
            (2,  (23, 1)),
            (3,  (22, 1)),
            (4,  (21,  1)),
            (5,  (20,  1)),
            (6,  (19,  1)),
            (7,  (18,  1)),
            (8,  (17,  1)),
            (9,  (16,  1)),
            (10, (15,  1)),
            (11, (14,  1)),
            (12, (13,  1)),
            (13, (12, 0)),
            (14, (11, 0)),
            (15, (10, 0)),
            (16, (9,  0)),
            (17, (8,  0)),
            (18, (7,  0)),
            (19, (6,  0)),
            (20, (5,  0)),
            (21, (4,  0)),
            (22, (3,  0)),
            (23, (2,  0)),
            (24, (1,  0)),
        ], 2, "can2", 17652))]
    /// Assert that the channel maps are serialised correctly to ensure 
    /// that the wiring harness is mapped to the software so messages 
    /// still get sent to the expected output.
    ///
    /// * `params`: Vector of channel maps.
    fn test_write_component_config_to_file_with_channel_maps(
          #[case] params:   (Vec<(u8, (u8, u8))>, u8, &str, i32),
    ) {
        // TODO: The clippy lint seems to be ignored due to the macro.
        //       It is an ugly type so reworking it is probably the 
        //       right choice, good first issue.

        let mut channel_map: HashMap<u8, (u8, u8)> = HashMap::new();

        for entry in params.0 {
            channel_map.insert(entry.0, entry.1);
        }

        let config = CropBedPowerConfig::new(
                params.1,
                String::from(params.2),
                params.3,
                Some(channel_map.clone()),
            )
            .add_pdm_config_file("./config/devices/crop_bed/pdm_0.yaml", 0)
            .add_pdm_config_file("./config/devices/crop_bed/pdm_1.yaml", 1);

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(Path::new(&format!("{}/config/components/crop_bed/actuating/power/crop_bed_power_{}.yaml", env!("CARGO_MANIFEST_DIR"), params.1)))
                .expect("Faile to open file");
            serde_yaml::to_writer(file, &config).expect("Failed to write yaml");
    }

    #[test]
    #[serial]
    fn test_write_component_config_to_file() {
        let pdm_config_ids: Vec<(u8, &str, i32)> = vec![(0, "can0", 17650)];

        for (id, interface, port) in pdm_config_ids {
            let config = CropBedPowerConfig::new(id, String::from(interface), port, None)
                .add_pdm_config_file("./config/devices/crop_bed/pdm_0.yaml", 0)
                .add_pdm_config_file("./config/devices/crop_bed/pdm_1.yaml", 1);

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(Path::new(&format!(
                    "{}/config/components/crop_bed/actuating/power/crop_bed_power_{id}.yaml",
                    env!("CARGO_MANIFEST_DIR")
                )))
                .expect("Faile to open file");
            serde_yaml::to_writer(file, &config).expect("Failed to write yaml");
        }
    }

    #[test]
    #[serial]
    fn test_read_component_config_to_file() {
        let pdm_config_ids: Vec<(u8, &str, i32)> =
            vec![(0, "can0", 17650), (1, "can1", 17651), (2, "can2", 17652)];

        for (id, interface, port) in pdm_config_ids {
            let write_config = CropBedPowerConfig::new(id, String::from(interface), port, None)
                .add_pdm_config_file("./config/devices/crop_bed/pdm_0.yaml", 0)
                .add_pdm_config_file("./config/devices/crop_bed/pdm_1.yaml", 1);

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(Path::new(&format!(
                    "{}/config/components/crop_bed/actuating/power/crop_bed_power_{id}_no_map.yaml",
                    env!("CARGO_MANIFEST_DIR")
                )))
                .expect("Faile to open file");

            serde_yaml::to_writer(file, &write_config).expect("Failed to write yaml");

            let read_config = CropBedPowerConfig::from_file(Path::new(&format!(
                "{}/config/components/crop_bed/actuating/power/crop_bed_power_{id}_no_map.yaml",
                env!("CARGO_MANIFEST_DIR")
            )));

            assert_eq!(
                write_config, read_config,
                "Failed to read write array config"
            );
        }
    }
}
