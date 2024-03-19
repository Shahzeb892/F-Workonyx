/// Devices are the atomic units that can be combined together
/// into components. Their core responsibilities do not change 
/// based on location, name etc.
pub mod hardware {
    /// Device interface for the network cameras.
    pub mod camera;
    /// Device interface for the pdm.
    pub mod pdm;
}

/// TODO: Not utilised as yet.
pub mod software {}
