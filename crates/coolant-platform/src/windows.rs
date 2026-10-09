//! Placeholder backend: compiles and reports every capability as
//! unsupported until it is implemented.

use crate::*;

pub struct Windows;

impl Backend for Windows {
    fn name(&self) -> &'static str {
        "windows"
    }
    fn machine(&self) -> Machine {
        Machine { os: "windows".into(), ..Default::default() }
    }
    fn processes(&self) -> Result<Vec<Process>> {
        Err(Error::Unsupported)
    }
    fn cpu(&self) -> Result<CpuTimes> {
        Err(Error::Unsupported)
    }
    fn sensors(&self) -> Result<Box<dyn SensorReader>> {
        Err(Error::Unsupported)
    }
    fn spawn_counter(&self) -> Result<Box<dyn SpawnCounter>> {
        Err(Error::Unsupported)
    }
}
