/// Components that are placed within a crop bed module on the machine.
pub mod crop_bed {
    /// Components that provide sensing capability.
    pub mod sensing {
        /// The camera array which holds several camera devices.
        pub mod camera_array;
    }
    /// Components that provide actuation capability.
    pub mod actuating {
        /// The PDM controls for lighting.
        pub mod lighting;
        /// The PDM controls for solenoids and power.
        pub mod power;
    }
}

/// Helpful prelude when working with components.
pub mod prelude {
    pub use crate::components::crop_bed::actuating::lighting::*;
    pub use crate::components::crop_bed::actuating::power::*;
    pub use crate::components::crop_bed::sensing::camera_array::*;
}
