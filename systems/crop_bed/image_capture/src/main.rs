//! Image capture binary.
use clap::Parser;
use onyx::components::prelude::*;

/// Arguments required for starting the program from the command line.
#[derive(Parser, Debug)]
struct Args {
    /// Path to the config file for the Lighting Component.
    #[arg(short, long)]
    filepath: String,
}

fn main() {
    let args = Args::parse();
    let component = CameraArray::from_config_file(args.filepath);
    let (_handles, _signal) = CameraArrayController::start(component);
    #[allow(clippy::empty_loop)]
    loop {
        // busy loop implement http listener here which can act as the HMI controller
    }
}
