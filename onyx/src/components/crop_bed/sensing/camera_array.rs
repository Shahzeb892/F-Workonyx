use crate::devices::hardware::camera::{
    CameraController, DevicePayload, OnyxCamera, OnyxCameraConfig,
};
use ringbuffer::{AllocRingBuffer, RingBuffer};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::Display,
    fs::create_dir_all,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc, Arc, Barrier},
    thread::{self, JoinHandle},
};
use uuid::Uuid;

/// Camera handle is generated when starting a device from
/// within a component. Allows threads to be stopped in a
/// consistent manner.
#[allow(dead_code)]
pub struct CameraHandle {
    /// The spawned thread handle that needs to be cleaned up.
    join_handle: Option<JoinHandle<()>>,
    /// Thread safe signal to gracefully shutdown a separate thread.
    stop_signal: Option<Arc<AtomicBool>>,
}

/// Type safe device position, helpful if devices are added to different parts 
/// of the machine, but perform different functions.
#[derive(Eq, PartialEq, Hash, Copy, Clone, Deserialize, Debug, PartialOrd, Ord)]
pub enum DevicePosition {
    /// As per the bill of materials.
    // TODO: Ensure common naming convention is reached between all 
    //       engineering disciplines. This did not happen in the 
    //       previous iteration unfortunately. 
    BedPosition(u8),
}

impl Display for DevicePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DevicePosition::BedPosition(idx) => write!(f, "bed_location_{}", idx),
        }
    }
}

/// As with all elements in the onyx system, a configuration struct
/// is consumed to create the necessary component, which in turn
/// control the devices that are composed together.
#[derive(Deserialize, Debug, Clone, Serialize, PartialEq)]
pub struct CameraArrayConfig {
    /// Determine which crop bed the component is connected to.
    crop_bed_id: u8,
    /// Where to store images on disk.
    image_path: String,
    /// Map of config files used to generate the cameras in the array.
    camera_config_files: HashMap<u8, PathBuf>,
}

impl CameraArrayConfig {
    /// Create a new empty camera config.
    ///
    /// * `image_path`: path to config file.
    /// * `crop_bed_id`: id of the crop bed module etc. (0, 1, 2)
    // TODO: Create a set of enums for module numbering and naming
    //       and set it in line with mechanical, electrical such
    //       as Module::Left, Module::Centre, Module::Right.
    pub fn new(image_path: String, crop_bed_id: u8) -> Self {
        Self {
            image_path,
            crop_bed_id,
            camera_config_files: HashMap::new(),
        }
    }

    /// Dynamically add additional camera config to the component that
    /// can be initiated during the component build.
    ///
    /// * `filepath`: path to camera config
    /// * `bed_position`: position in line with bill of materials.
    pub fn add_camera_config_file<F>(mut self, filepath: F, bed_position_idx: u8) -> Self
    where
        F: AsRef<OsStr>,
    {
        self.camera_config_files
            .insert(bed_position_idx, (&filepath).into());
        self
    }

    /// Create a camera array component from a config file.
    ///
    /// * `filepath`: path to camera array config config.
    pub fn from_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let file = Path::new(&filepath);
        let camera_array_config = if file.is_file() {
            let config_file = config::Config::builder()
                .add_source(config::File::new(
                    &file.to_string_lossy(),
                    config::FileFormat::Yaml,
                ))
                .build()
                .expect("Failed read config");
            config_file
                .try_deserialize::<CameraArrayConfig>()
                .expect("Failed to parse config file into struct")
        } else {
            panic!("Could not locate the config file {:?}", file);
        };
        camera_array_config
    }
}

/// Component that contains the individual cameras that are attached to
/// it. This can be scaled to either run all the cameras, or sections of
/// the cameras available on the machine. In the first iteration the set
/// of cameras were logically split up by crop beds; there are some small
/// issues with this approach as physically the machine has been designed
/// with differing module widths (i.e. centre is fixed while booms vary).
pub struct CameraArray {
    /// Unique id of the camera array.
    uuid: Uuid,
    /// Map of the thread handles and start stop atomics's.
    camera_handles: HashMap<Uuid, CameraHandle>,
    /// Map of the camera devices.
    cameras: HashMap<u8, OnyxCamera>,
    /// Parent save directory for the images.
    // TODO: Remove once port streaming is implemented.
    pub image_path: String,
    /// Crop bed id from the machine as per the bill of materials.
    crop_bed_id: u8,
}

impl CameraArray {
    /// Return the unique id of the camera array.
    pub fn get_uuid(&self) -> Uuid {
        self.uuid
    }

    /// Create camera array by consuming a config.
    ///
    /// * `config`: Specified camera array config
    pub fn new(config: CameraArrayConfig) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            image_path: config.image_path.clone(),
            crop_bed_id: config.crop_bed_id,
            camera_handles: HashMap::new(),
            cameras: Self::build_from_config(config),
        }
    }

    /// Create a camera array component by ingesting a config file.
    ///
    /// * `filepath`: filepath to the config.
    pub fn from_config_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let config = CameraArrayConfig::from_file(filepath);
        Self::new(config)
    }

    /// Build the devices linked to the component, in this case the individual
    /// cameras within the `CameraArray`. This is a helper function.
    ///
    /// * `config`: `CameraArrayConfig`
    fn build_from_config(config: CameraArrayConfig) -> HashMap<u8, OnyxCamera> {
        let mut cameras = HashMap::new();

        for (bed_position, camera_config_file) in config.camera_config_files {
            let camera = OnyxCamera::new(OnyxCameraConfig::from_file(camera_config_file));
            cameras.insert(bed_position, camera);
        }
        cameras
    }
}

/// Unit struct to link component controller behaviour, all components will
/// need some type of behaviour and it is easier to detach this behaviour
/// from requiring owned state. Rather pass it to functions that do the work.
pub struct CameraArrayController;

impl CameraArrayController {
    /// Start the cameras in their own threads.
    ///
    /// * `camera_array`: Component containing initialised cameras.
    // TODO: Using separate threads for networks cameras is an interesting choice considering
    //       much of the time the device will be in a hold state, so the thread will be context
    //       switching. The obvious alternative is to change this to async, however at the time
    //       the underlying aravis library did not implement any futures capability, and there
    //       was not enough time to write and contribute an async version.
    pub fn start(
        mut camera_array: CameraArray,
    ) -> (JoinHandle<AllocRingBuffer<JoinHandle<()>>>, Arc<AtomicBool>) {
        let nthread = camera_array.cameras.len();
        let barrier = Arc::new(Barrier::new(nthread));
        let stop_signal = Arc::new(AtomicBool::new(false));
        let (device_channel_tx, device_channel_rx) = mpsc::channel::<DevicePayload>();

        let path = PathBuf::from(format!(
            "{}/{}",
            camera_array.image_path, camera_array.crop_bed_id
        ));
        create_dir_all(&path).expect("Failed to create filepath");


        // TODO: Should be as simple as dropping the into_iter however this update 
        //       was explicitly no code changes due to upcoming tests on farms.
        //       Good first issue.
        #[allow(clippy::explicit_into_iter_loop)]
        for (bed_position, mut camera) in camera_array.cameras.into_iter() {
            create_dir_all(&path.join(bed_position.to_string()))
                .expect("Failed to create bed position path");
            let camera_uuid = camera.get_uuid();
            // Set up the requirements for the threads to operate.
            // lots of clones as new thread will take ownership.
            let thread_barrier = barrier.clone();
            let thread_stop_signal = stop_signal.clone();
            let caller_stop_signal = stop_signal.clone();
            let thread_device_sender_tx = device_channel_tx.clone();

            camera.set_location_id(bed_position);

            let device_handle = thread::spawn(move || {
                CameraController::start(
                    camera,
                    thread_stop_signal,
                    thread_barrier,
                    thread_device_sender_tx,
                );
            });

            camera_array.camera_handles.insert(
                camera_uuid,
                CameraHandle {
                    join_handle: Some(device_handle),
                    stop_signal: Some(caller_stop_signal),
                },
            );
        }

        // TODO: write out the device signals to either another object or to the struct.
        // Issue here is that the vector can grow infinitely so we need to get rid of
        // some of the successful thread join handles. Currently using a ring buffer
        // to discard join handles that leak  over the total length of the thread handle
        // storage. An alternate way try to implement this is to add another MPSC and use
        // the try_recv function to test if there are any threads that should be closed.
        // A very naive way would also be to just chuck these image writer join handles
        // away. Ultimately due to schedule / resourcing unable to spend time on this.

        let handles = thread::spawn(|| {
            let thread_path = Arc::new(path);

            let mut image_writer_handles_buffer = AllocRingBuffer::new(128);
            for payload in device_channel_rx {
                let image_path = thread_path.clone();
                let image_writer_handle = thread::spawn(move || {
                    let filename = image_path.join(payload.filename());
                    if let Err(e) = payload.image.save(&filename) {
                        println!("Failed to save image to path {:?} {e}", filename);
                    }
                });
                image_writer_handles_buffer.push(image_writer_handle);
            }
            image_writer_handles_buffer
        });
        (handles, stop_signal)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use serial_test::serial;
    use std::{fs::OpenOptions, time::Duration};

    #[test]
    #[serial]
    /// Test writing component configurations to a yaml file.
    fn test_write_component_config_to_file() {
        let camera_array_configs: Vec<(u8, [u8; 2])> = vec![(0, [0, 1]), (1, [2, 3]), (2, [4, 5])];

        for (crop_bed_id, camera_ids) in camera_array_configs {
            let config = CameraArrayConfig::new(String::from("./images"), crop_bed_id)
                .add_camera_config_file(
                    format!("./config/devices/crop_bed/camera_{}.yaml", camera_ids[0]),
                    0,
                )
                .add_camera_config_file(
                    format!("./config/devices/crop_bed/camera_{}.yaml", camera_ids[1]),
                    1,
                );

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(Path::new(&format!("{}/config/components/crop_bed/sensing/camera_array/crop_bed_camera_array_{crop_bed_id}.yaml", env!("CARGO_MANIFEST_DIR"))))
                .expect("Faile to open file");

            serde_yaml::to_writer(file, &config).expect("Failed to write yaml");
        }
    }

    #[test]
    #[serial]
    /// Test writing component configurations to a yaml file, and reading
    /// back to a type safe structure.
    fn test_read_write_component_config_to_file() {
        let write_config = CameraArrayConfig::new(String::from("./images"), 0)
            .add_camera_config_file(format!("./config/devices/crop_bed/camera_{}.yaml", 0), 0)
            .add_camera_config_file(format!("./config/devices/crop_bed/camera_{}.yaml", 1), 1);

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(Path::new(&format!(
                "{}/config/components/crop_bed/sensing/camera_array/crop_bed_camera_array_0.yaml",
                env!("CARGO_MANIFEST_DIR")
            )))
            .expect("Failed to open file");

        serde_yaml::to_writer(file, &write_config).expect("Failed to write yaml");

        let read_config = CameraArrayConfig::from_file(Path::new(&format!(
            "{}/config/components/crop_bed/sensing/camera_array/crop_bed_camera_array_0.yaml",
            env!("CARGO_MANIFEST_DIR")
        )));

        assert_eq!(
            write_config, read_config,
            "Failed to read write array config"
        );
    }

    #[test]
    /// Review how hash maps are serialised to yaml with serde.
    fn test_serde_hashmap_camera_configs() {
        let mut map: Option<HashMap<u8, String>> = Some(HashMap::new());

        map = if let Some(mut map) = map {
            map.insert(0, "TestFile_0.yaml".to_string());
            map.insert(1, "TestFile_1.yaml".to_string());
            Some(map)
        } else {
            None
        };

        let yaml_string = serde_yaml::to_string(&map).unwrap();
        let compare = serde_yaml::from_str::<Option<HashMap<u8, String>>>(&yaml_string).unwrap();
        assert_eq!(map, compare);
    }

    #[cfg_attr(not(feature = "hardware_test"), ignore)]
    #[test]
    #[serial]
    /// Test the builder pattern when reading a configuration file.
    fn test_camera_array_config_builder_from_file() {
        let config_file = format!(
            "{}/config/components/crop_bed/sensing/camera_array/crop_bed_array_test.yaml",
            env!("CARGO_MANIFEST_DIR")
        );
        let camera_array = CameraArray::from_config_file(config_file);
        assert!(camera_array.cameras.len() == 1);
    }

    #[test]
    #[serial]
    #[cfg_attr(not(feature = "hardware_test"), ignore)]
    /// Hardware test to check the correct number of images are captured 
    /// to meet the required FPS in the specification.
    fn test_camera_array_config_build_run_and_count_images() {
        let config_file = format!(
            "{}/config/components/crop_bed/sensing/camera_array/crop_bed_array_test.yaml",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut camera_array = CameraArray::from_config_file(config_file);
        camera_array.image_path = String::from("./test-outputs/component-tests/camera_array");

        let (handles, stop_signal) = CameraArrayController::start(camera_array);
        thread::sleep(Duration::from_secs(5));

        stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);

        let mut image_writers = handles
            .join()
            .expect("Unable to return image writer thread");

        for image_writer in image_writers.drain() {
            image_writer
                .join()
                .expect("Failed to shut down image writer.");
        }

        thread::sleep(Duration::from_secs(1));

        let total_images = std::fs::read_dir(format!(
            "{}/test-outputs/component-tests/camera_array/0/0",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("Failed to read dir")
        .count();

        assert!(
            (50_usize.abs_diff(total_images)) < 5,
            "Failed to generate the correct number of images @ 10 FPS"
        );
    }
}
