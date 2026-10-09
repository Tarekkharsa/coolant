//! Placeholder backend: compiles and reports every capability as
//! unsupported until it is implemented.

use crate::*;

pub struct Linux;

impl Backend for Linux {
    fn name(&self) -> &'static str {
        "linux"
    }
    fn machine(&self) -> Machine {
        Machine { os: "linux".into(), ..Default::default() }
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
