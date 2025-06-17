use mxpsu::MxSeries;
use mxpsu::error::MxError;

fn main() -> Result<(), MxError> {
    let mut psu = MxSeries::connect_serial("/dev/ttyACM0", 9600)?;

    psu.turn_on(1)?;
    psu.turn_off(1)?;

    println!("Channel 1 turned on successfully.");
    Ok(())
}