use mxpsu::MxSeries;
use mxpsu::error::MxError;

fn main() -> Result<(), MxError> {
    let mut psu = MxSeries::connect_serial("/dev/ttyACM0", 9600)?;

    psu.turn_on(1)?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    psu.turn_off(1)?;

    Ok(())
}