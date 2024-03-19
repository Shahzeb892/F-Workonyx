use crate::{
    devices::hardware::pdm::{Pdm, PdmConfig},
    messages::control::light::LightMessage,
};
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
};
use uuid::Uuid;

/// Configuration for the crop bed lighting using the utilities PDM.
// TODO: Extend config to identify which channels are actually going
//       to be connected to the lights, as this has not been properly 
//       wired or documented.
#[derive(Deserialize, Serialize, PartialEq, Eq, Debug)]
pub struct CropBedLightingConfig {
    /// Id the crop bed lighting is attached to.
    crop_bed_id: u8,
    /// Canbus interface name.
    canbus_id: String,
    /// Internal linux port the component will listen to messages for.
    port: i32,
    /// Map of config files used to set up the PDMs in the component.
    pdm_config_files: HashMap<u8, PathBuf>,
}

// TODO: Similar to others, extract out relevant methods to traits.
impl CropBedLightingConfig {
    /// Crop bed lighting configuration.
    ///
    /// * `crop_bed_id`: crop bed ids from [0 - 2]
    /// * `canbus_id`: String for the bus ie, can0.
    pub fn new(crop_bed_id: u8, canbus_id: String, port: i32) -> Self {
        Self {
            port,
            crop_bed_id,
            canbus_id,
            pdm_config_files: HashMap::new(),
        }
    }

    /// Add a PDM config file to the component this will be consumed
    /// when the component is created.
    ///
    /// * `filepath`: filepath to the config file
    /// * `pdm_id`: Address of the PDM, 30, 31, 32, 33
    pub fn add_pdm_config_file<F>(mut self, filepath: F, pdm_id: u8) -> Self
    where
        F: AsRef<OsStr>,
    {
        self.pdm_config_files.insert(pdm_id, (&filepath).into());
        self
    }

    /// Build the config by reading a file, this is a helper function.
    ///
    /// * `filepath`: path to config.
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
                .try_deserialize::<CropBedLightingConfig>()
                .expect("Failed to parse config file into struct")
        } else {
            panic!("Could not locate the config file {:?}", file);
        }
    }
}

/// Component that houses the PDM devices which are configured to provide 
/// lighting for the crop bed.
#[allow(dead_code)]
pub struct CropBedLighting {
    /// Unique id of the component.
    uuid: Uuid,
    /// Crop bed id the component is tied to.
    crop_bed_id: u8,
    /// Canbus interface name.
    canbus_id: String,
    /// Map of the PDMs this component managers.
    pdms: HashMap<u8, Pdm>,
    /// Internal linux port that this component listens to.
    port: i32,
}

impl CropBedLighting {
    /// Generate a new component by consuming a config.
    ///
    /// * `config`: `CropBedLightingConfig`
    pub fn new(config: CropBedLightingConfig) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            port: config.port,
            crop_bed_id: config.crop_bed_id,
            canbus_id: config.canbus_id.clone(),
            pdms: Self::build_from_config(config),
        }
    }

    /// Generate a new component by consuming the config stored
    /// in a file.
    ///
    /// * `filepath`: filepath to a config.
    pub fn from_config_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let config = CropBedLightingConfig::from_file(filepath);
        Self::new(config)
    }

    /// Internal helper function to create a component from a config struct.
    ///
    /// * `config`: Struct with config details.
    fn build_from_config(config: CropBedLightingConfig) -> HashMap<u8, Pdm> {
        let mut pdms = HashMap::new();
        for (bed_position, pdm_config_file) in config.pdm_config_files {
            let pdm = Pdm::new(PdmConfig::from_file(pdm_config_file));
            pdms.insert(bed_position, pdm);
        }
        pdms
    }
}

/// Unit struct for controlling the lighting component.
pub struct CropBedLightingController;

impl CropBedLightingController {
    /// Start the component.
    ///
    /// * `crop_bed_power`: consume to components
    // TODO: move this to pass by reference.
    pub async fn start(mut crop_bed_power: CropBedLighting) {
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

        // TODO: Remove the continue, picked up with more strict clippy linting.
        //       very straight forward. Good first issue. Good first issue.
        #[allow(clippy::needless_continue)]
        loop {
            // TODO: review this busy loop.
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

/// Handle new connection and stay connected to keep reading the bytes sent over the wire.
///
/// * `socket`: internal linux socket.
/// * `power`:  component.
async fn handle_connection(mut socket: TcpStream, power: Arc<Mutex<CropBedLighting>>) {
    let (read_stream, _) = socket.split();
    let mut read_stream = BufReader::new(read_stream);
    let mut data = Vec::new();

    loop {
        // TODO: break loop if connection is ended,log issue if terminated prematurely.
        data.clear();
        let bytes_read = read_stream
            .read_until(b'\n', &mut data)
            .await
            .expect("Failed to read buffer");

        if bytes_read != 0 {
            match serde_json::from_slice::<LightMessage>(&data) {
                // TODO: add in logs for wrong crop bed, camera ids.
                Ok(message) => {
                    println!("Received a message {:?}", message);

                    let gaurd = power.lock().await;

                    if message.is_on {
                        if let Some(pdm) = gaurd.pdms.get(&0) {
                            pdm.driver
                                .actuate_channels(17, message.channels, 100.0)
                                .await;
                        }
                    } else if let Some(pdm) = gaurd.pdms.get(&0) {
                        pdm.driver.actuate_channels(17, message.channels, 0.0).await;
                    }
                    // Make sure to drop the guard strait after using in the loop.
                    drop(gaurd);
                }
                Err(e) => {
                    println!("Received a malformed request {:?}, data: {:?}", e, &data);
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs::OpenOptions;

    #[test]
    #[serial]
    fn test_write_component_config_to_file() {
        let pdm_config_ids: Vec<(u8, &str, i32)> = vec![(0, "can3", 17653)];

        for (id, interface, port) in pdm_config_ids {
            let config = CropBedLightingConfig::new(id, String::from(interface), port)
                .add_pdm_config_file("./config/devices/crop_bed/pdm_utilities.yaml", 0);

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(Path::new(&format!(
                    "{}/config/components/crop_bed/actuating/lighting/crop_bed_lighting.yaml",
                    env!("CARGO_MANIFEST_DIR")
                )))
                .expect("Faile to open file");
            serde_yaml::to_writer(file, &config).expect("Failed to write yaml");
        }
    }

    #[test]
    #[serial]
    fn test_read_component_config_to_file() {
        let pdm_config_ids: Vec<(u8, &str, i32)> = vec![(0, "can3", 17653)];

        for (id, interface, port) in pdm_config_ids {
            let write_config = CropBedLightingConfig::new(id, String::from(interface), port)
                .add_pdm_config_file("./config/devices/crop_bed/pdm_utilities.yaml", 0);

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(Path::new(&format!(
                    "{}/config/components/crop_bed/actuating/lighting/crop_bed_lighting.yaml",
                    env!("CARGO_MANIFEST_DIR")
                )))
                .expect("Faile to open file");
            serde_yaml::to_writer(file, &write_config).expect("Failed to write yaml");
            let read_config = CropBedLightingConfig::from_file(Path::new(&format!(
                "{}/config/components/crop_bed/actuating/lighting/crop_bed_lighting.yaml",
                env!("CARGO_MANIFEST_DIR")
            )));
            assert_eq!(
                write_config, read_config,
                "Failed to read write array config"
            );
        }
    }
}
