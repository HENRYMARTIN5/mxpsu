pub mod connection;
pub mod error;

use connection::Connection;
use error::MxError;
use phf::phf_map;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

static EXECUTION_ERROR_CODES: phf::Map<i32, (&'static str, &'static str)> = phf_map! {
    0i32 => ("OK", "No error has occurred since this register was last read."),
    100i32 => ("NumericError", "The parameter value sent was outside the permitted range for the command in the present circumstances."),
    102i32 => ("RecallError", "A recall of set up data has been requested but the store specified does not contain any data."),
    103i32 => ("CommandInvalid", "The command is recognised but is not valid in the current circumstances. Typical examples would be trying to change V2 directly while the outputs are in voltage tracking mode with V1 as the master."),
    104i32 => ("RangeChangeError", "An operation requiring a range change was requested but could not be completed. Typically this occurs because >0.5V was still present on output 1 and/or output 2 terminals at the time the command was executed."),
    200i32 => ("AccessDenied", "An attempt was made to change the instrument's settings from an interface which is locked out of write privileges by a lock held by another interface.")
};

/// Represents the state of the Event Status Register.
pub enum ESRValue {
    Integer(u8),
    BinaryString(String),
}

/// Actions for multi-channel on/off operations.
#[derive(Debug, Clone, Copy)]
pub enum MultiActionType {
    Quick,
    Never,
    Delay,
}

/// Configuration for a multi-channel operation on a specific channel.
#[derive(Debug, Clone, Copy)]
pub enum MultiOperationConfig {
    /// Turn on/off quickly or never. `true` for QUICK, `false` for NEVER.
    Action(bool),
    /// Turn on/off with a specified delay.
    DelayMs(u16),
}

/// Averaging settings for current meter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeterAveraging {
    On,
    Off,
    Low,
    Med,
    High,
}

impl MeterAveraging {
    fn as_str(&self) -> &'static str {
        match self {
            MeterAveraging::On => "ON",
            MeterAveraging::Off => "OFF",
            MeterAveraging::Low => "LOW",
            MeterAveraging::Med => "MED",
            MeterAveraging::High => "HIGH",
        }
    }
}


/// Main struct for interacting with an MX Series power supply.
pub struct MxSeries {
    connection: Box<dyn Connection>,
}

impl MxSeries {
    /// Creates a new `MxSeries` instance with a socket connection.
    #[cfg(feature = "socket")]
    pub fn connect_socket(address: &str) -> Result<Self, MxError> {
        let conn = connection::SocketConnection::new(address)?;
        Ok(MxSeries {
            connection: Box::new(conn),
        })
    }

    /// Creates a new `MxSeries` instance with a serial connection.
    #[cfg(feature = "serial")]
    pub fn connect_serial(port_name: &str, baud_rate: u32) -> Result<Self, MxError> {
        let conn = connection::SerialConnection::new(port_name, baud_rate)?;
        Ok(MxSeries {
            connection: Box::new(conn),
        })
    }

    /// Sets the communication timeout for the connection.
    pub fn set_timeout(&mut self, duration: Duration) -> Result<(), MxError> {
        self.connection.set_timeout(duration)
    }

    fn _check_event_status_register(&mut self, command_sent: &str) -> Result<(), MxError> {
        // Query the raw ESR value. *ESR? also clears it.
        let esr_reply = match self.connection.query("*ESR?") {
            Ok(reply) => reply,
            Err(e) => return Err(MxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to query *ESR?: {} (Original command: {})", e, command_sent),
            ))),
        };

        let status_val = match esr_reply.trim().parse::<u8>() {
            Ok(val) => val,
            Err(_) => return Err(MxError::Parse(format!(
                "Could not parse ESR value: '{}'. Original command: {}",
                esr_reply, command_sent
            ))),
        };

        // Bit 7 - Power On (128) - Ignored as it's normal after power on.
        // Bit 6 - User Request (64) - Not used by these commands.
        // Bit 1 - Not used (2)
        // Bit 0 - Operation Complete (1) - Set by *OPC, not an error.

        if status_val & 0b00100000 != 0 { // Bit 5 - Command Error
            return Err(MxError::CommandError(format!(
                "Syntax error in command or parameter. Command: '{}'", command_sent
            )));
        }
        if status_val & 0b00010000 != 0 { // Bit 4 - Execution Error
            let eer_str = self.connection.query("EER?")?.trim().to_string();
            let error_code = eer_str.parse::<i32>()
                .map_err(|_| MxError::Parse(format!("Failed to parse EER value: {}", eer_str)))?;
            
            if let Some((err_type, err_msg)) = EXECUTION_ERROR_CODES.get(&error_code) {
                return Err(MxError::ExecutionError {
                    code: error_code,
                    error_type: err_type.to_string(),
                    description: err_msg.to_string(),
                });
            } else {
                return Err(MxError::UndefinedDeviceErrorCode(error_code, command_sent.to_string()));
            }
        }
        if status_val & 0b00001000 != 0 { // Bit 3 - Device Dependent Error (Verify Timeout on MX)
            return Err(MxError::VerifyTimeoutError(format!(
                "Verify timeout or device dependent error. Command: '{}'", command_sent
            )));
        }
        if status_val & 0b00000100 != 0 { // Bit 2 - Query Error
            return Err(MxError::QueryError(format!(
                "Query error (e.g., attempt to read without sending command). Command: '{}'", command_sent
            )));
        }
        Ok(())
    }

    fn _write_and_check(&mut self, command: &str) -> Result<(), MxError> {
        self.connection.write_command(command)?;
        // A small delay can be crucial for the instrument to process the command
        // before its status registers are updated and checked.
        thread::sleep(Duration::from_millis(50)); // Adjust as needed
        self._check_event_status_register(command)
    }

    fn _query_and_check(&mut self, command: &str) -> Result<String, MxError> {
        match self.connection.query(command) {
            Ok(response) => {
                // Even on successful query, check ESR for any latent errors from this command.
                // This behavior might differ from the Python version's `except` block,
                // which only checks ESR if the query itself fails at the communication level.
                // However, some devices might execute a query, return data, but still set an ESR bit.
                // For safety, we check. If this causes issues, it can be removed.
                // thread::sleep(Duration::from_millis(50)); // If needed before ESR check
                // self._check_event_status_register(command)?; // Potentially too strict
                Ok(response.trim().to_string())
            }
            Err(e) => {
                // If query itself fails (e.g. timeout, IO error), then check ESR.
                // This is closer to the Python version's logic.
                match self._check_event_status_register(command) {
                    Ok(_) => Err(e), // ESR was clear, so original communication error stands
                    Err(esr_err) => Err(esr_err), // ESR had an error, report that as it's more specific
                }
            }
        }
    }

    /// Send the clear, `*CLS`, command. This clears status registers.
    pub fn clear(&mut self) -> Result<(), MxError> {
        self.connection.write_command("*CLS")
        // Do not call _check_event_status_register here as *CLS clears it.
    }

    /// Decrement the current limit by step size of the output channel.
    pub fn decrement_current(&mut self, channel: u8) -> Result<(), MxError> {
        self._write_and_check(&format!("DECI{}", channel))
    }

    /// Decrement the voltage by step size of the output channel.
    pub fn decrement_voltage(&mut self, channel: u8, verify: bool) -> Result<(), MxError> {
        let command = format!("DECV{}{}", channel, if verify { "V" } else { "" });
        self._write_and_check(&command)
    }

    /// Read and clear the standard event status register.
    pub fn event_status_register(&mut self, as_integer: bool) -> Result<ESRValue, MxError> {
        let val_str = self.connection.query("*ESR?")?; // *ESR? reads and clears
        let value = val_str.trim().parse::<u8>().map_err(|e| {
            MxError::Parse(format!("Failed to parse ESR value '{}': {}", val_str, e))
        })?;
        if as_integer {
            Ok(ESRValue::Integer(value))
        } else {
            Ok(ESRValue::BinaryString(format!("{:08b}", value)))
        }
    }

    /// Get the output current of the output channel.
    pub fn get_current(&mut self, channel: u8) -> Result<f32, MxError> {
        let reply = self._query_and_check(&format!("I{}O?", channel))?;
        // Reply format: "1.234A"
        if let Some(val_str) = reply.strip_suffix('A') {
            val_str.parse::<f32>().map_err(MxError::from)
        } else {
            Err(MxError::Parse(format!("Unexpected format for get_current (I{}O?): '{}'", channel, reply)))
        }
    }

    /// Get the current limit of the output channel.
    pub fn get_current_limit(&mut self, channel: u8) -> Result<f32, MxError> {
        let reply = self._query_and_check(&format!("I{}?", channel))?;
        // Reply format: "I1 0.500"
        let parts: Vec<&str> = reply.split_whitespace().collect();
        if parts.len() == 2 {
            parts[1].parse::<f32>().map_err(MxError::from)
        } else {
            Err(MxError::Parse(format!("Unexpected format for get_current_limit (I{}?): '{}'", channel, reply)))
        }
    }

    /// Get the current limit step size of the output channel.
    pub fn get_current_step_size(&mut self, channel: u8) -> Result<f32, MxError> {
        let reply = self._query_and_check(&format!("DELTAI{}?", channel))?;
        // Reply format: "DELTAI1 0.010"
        let parts: Vec<&str> = reply.split_whitespace().collect();
        if parts.len() == 2 {
            parts[1].parse::<f32>().map_err(MxError::from)
        } else {
            Err(MxError::Parse(format!("Unexpected format for get_current_step_size (DELTAI{}?): '{}'", channel, reply)))
        }
    }
    
    /// Get the over-current protection trip point of the output channel.
    pub fn get_over_current_protection(&mut self, channel: u8) -> Result<Option<f32>, MxError> {
        let reply = self._query_and_check(&format!("OCP{}?", channel))?;
        // Reply format: "OCP1 1.500" or "OCP1 OFF"
        if reply.to_uppercase().ends_with("OFF") {
            Ok(None)
        } else {
            let parts: Vec<&str> = reply.split_whitespace().collect();
            if parts.len() == 2 {
                parts[1].parse::<f32>().map(Some).map_err(MxError::from)
            } else {
                Err(MxError::Parse(format!("Unexpected format for get_over_current_protection (OCP{}?): '{}'", channel, reply)))
            }
        }
    }

    /// Get the over-voltage protection trip point of the output channel.
    pub fn get_over_voltage_protection(&mut self, channel: u8) -> Result<Option<f32>, MxError> {
        let reply = self._query_and_check(&format!("OVP{}?", channel))?;
        // Reply format: "OVP1 30.50" or "OVP1 OFF"
         if reply.to_uppercase().ends_with("OFF") {
            Ok(None)
        } else {
            let parts: Vec<&str> = reply.split_whitespace().collect();
            if parts.len() == 2 {
                parts[1].parse::<f32>().map(Some).map_err(MxError::from)
            } else {
                Err(MxError::Parse(format!("Unexpected format for get_over_voltage_protection (OVP{}?): '{}'", channel, reply)))
            }
        }
    }

    /// Get the output voltage of the output channel.
    pub fn get_voltage(&mut self, channel: u8) -> Result<f32, MxError> {
        let reply = self._query_and_check(&format!("V{}O?", channel))?;
        // Reply format: "5.000V"
        if let Some(val_str) = reply.strip_suffix('V') {
            val_str.parse::<f32>().map_err(MxError::from)
        } else {
             Err(MxError::Parse(format!("Unexpected format for get_voltage (V{}O?): '{}'", channel, reply)))
        }
    }

    /// Get the output voltage range index of the output channel.
    pub fn get_voltage_range(&mut self, channel: u8) -> Result<i32, MxError> {
        let reply = self._query_and_check(&format!("VRANGE{}?", channel))?;
        // Reply format: "1" (integer)
        reply.parse::<i32>().map_err(MxError::from)
    }

    /// Get the set-point voltage of the output channel.
    pub fn get_voltage_setpoint(&mut self, channel: u8) -> Result<f32, MxError> {
        let reply = self._query_and_check(&format!("V{}?", channel))?;
        // Reply format: "V1 5.000"
        let parts: Vec<&str> = reply.split_whitespace().collect();
        if parts.len() == 2 {
            parts[1].parse::<f32>().map_err(MxError::from)
        } else {
            Err(MxError::Parse(format!("Unexpected format for get_voltage_setpoint (V{}?): '{}'", channel, reply)))
        }
    }

    /// Get the voltage step size of the output channel.
    pub fn get_voltage_step_size(&mut self, channel: u8) -> Result<f32, MxError> {
        let reply = self._query_and_check(&format!("DELTAV{}?", channel))?;
        // Reply format: "DELTAV1 0.010"
        let parts: Vec<&str> = reply.split_whitespace().collect();
        if parts.len() == 2 {
            parts[1].parse::<f32>().map_err(MxError::from)
        } else {
            Err(MxError::Parse(format!("Unexpected format for get_voltage_step_size (DELTAV{}?): '{}'", channel, reply)))
        }
    }

    /// Get the voltage tracking mode of the unit.
    pub fn get_voltage_tracking_mode(&mut self) -> Result<i32, MxError> {
        let reply = self._query_and_check("CONFIG?")?;
        // Reply format: "0" (integer)
        reply.parse::<i32>().map_err(MxError::from)
    }

    /// Increment the current limit by step size of the output channel.
    pub fn increment_current(&mut self, channel: u8) -> Result<(), MxError> {
        self._write_and_check(&format!("INCI{}", channel))
    }

    /// Increment the voltage by step size of the output channel.
    pub fn increment_voltage(&mut self, channel: u8, verify: bool) -> Result<(), MxError> {
        let command = format!("INCV{}{}", channel, if verify { "V" } else { "" });
        self._write_and_check(&command)
    }

    /// Check if the output channel is on or off.
    pub fn is_output_on(&mut self, channel: u8) -> Result<bool, MxError> {
        let reply = self._query_and_check(&format!("OP{}?", channel))?;
        // Reply format: "1" or "0"
        match reply.trim() {
            "1" => Ok(true),
            "0" => Ok(false),
            _ => Err(MxError::Parse(format!("Unexpected reply for is_output_on (OP{}?): '{}'", channel, reply))),
        }
    }

    /// Turn the output channel on.
    pub fn turn_on(&mut self, channel: u8) -> Result<(), MxError> {
        self._write_and_check(&format!("OP{} 1", channel))
    }

    /// Turn multiple output channels on (the Multi-On feature).
    pub fn turn_on_multi(&mut self, options: Option<HashMap<u8, MultiOperationConfig>>) -> Result<(), MxError> {
        if let Some(opts) = options {
            for (channel, config) in opts {
                match config {
                    MultiOperationConfig::Action(enable_quick) => {
                        self.set_multi_on_action(channel, if enable_quick { MultiActionType::Quick } else { MultiActionType::Never })?;
                    }
                    MultiOperationConfig::DelayMs(ms) => {
                        self.set_multi_on_action(channel, MultiActionType::Delay)?;
                        self.set_multi_on_delay(channel, ms)?;
                        thread::sleep(Duration::from_millis(100)); // As per Python code
                    }
                }
            }
        }
        self._write_and_check("OPALL 1")
    }

    /// Turn the output channel off.
    pub fn turn_off(&mut self, channel: u8) -> Result<(), MxError> {
        self._write_and_check(&format!("OP{} 0", channel))
    }

    /// Turn multiple output channels off (the Multi-Off feature).
    pub fn turn_off_multi(&mut self, options: Option<HashMap<u8, MultiOperationConfig>>) -> Result<(), MxError> {
        if let Some(opts) = options {
            for (channel, config) in opts {
                match config {
                    MultiOperationConfig::Action(enable_quick) => {
                        self.set_multi_off_action(channel, if enable_quick { MultiActionType::Quick } else { MultiActionType::Never })?;
                    }
                    MultiOperationConfig::DelayMs(ms) => {
                        self.set_multi_off_action(channel, MultiActionType::Delay)?;
                        self.set_multi_off_delay(channel, ms)?;
                        thread::sleep(Duration::from_millis(100)); // As per Python code
                    }
                }
            }
        }
        self._write_and_check("OPALL 0")
    }

    /// Recall the settings of the output channel from the store.
    pub fn recall(&mut self, channel: u8, index: u8) -> Result<(), MxError> {
        if index > 49 {
            return Err(MxError::InvalidParameter("Store index must be 0-49.".to_string()));
        }
        self._write_and_check(&format!("RCL{} {}", channel, index))
    }

    /// Recall the settings for all output channels from the store.
    pub fn recall_all(&mut self, index: u8) -> Result<(), MxError> {
        if index > 49 { // Manual implies *SAV/*RCL use same range as SAVx/RCLx
            return Err(MxError::InvalidParameter("Store index must be 0-49.".to_string()));
        }
        // Python code has *SAV here, but for recall it should be *RCL
        // Manual for MX100TP: "*RCL n Recalls settings for all outputs from store n."
        // Manual for MX100TP: "*SAV n Saves settings of all outputs to store n."
        // The Python code seems to have a typo here, using *SAV for recall_all.
        // Correcting to *RCL for recall_all.
        self._write_and_check(&format!("*RCL {}", index))
    }

    /// Send the reset, `*RST`, command.
    pub fn reset(&mut self) -> Result<(), MxError> {
        self.connection.write_command("*RST")?;
        // *RST can take some time. A delay might be prudent before subsequent commands.
        thread::sleep(Duration::from_millis(500)); // Adjust as needed
        Ok(())
    }

    /// Attempt to clear all trip conditions.
    pub fn reset_trip(&mut self) -> Result<(), MxError> {
        self._write_and_check("TRIPRST")
    }

    /// Save the present settings of the output channel to the store.
    pub fn save(&mut self, channel: u8, index: u8) -> Result<(), MxError> {
        if index > 49 {
            return Err(MxError::InvalidParameter("Store index must be 0-49.".to_string()));
        }
        self._write_and_check(&format!("SAV{} {}", channel, index))
    }

    /// Save the settings of all output channels to the store.
    pub fn save_all(&mut self, index: u8) -> Result<(), MxError> {
        if index > 49 {
            return Err(MxError::InvalidParameter("Store index must be 0-49.".to_string()));
        }
        // Python code has *RCL here, but for save it should be *SAV.
        // Correcting to *SAV for save_all.
        self._write_and_check(&format!("*SAV {}", index))
    }

    /// Set the current limit of the output channel.
    pub fn set_current_limit(&mut self, channel: u8, value: f32) -> Result<(), MxError> {
        self._write_and_check(&format!("I{} {:.3}", channel, value))
    }

    /// Set the current meter measurement averaging of the output channel.
    pub fn set_current_meter_averaging(&mut self, channel: u8, value: MeterAveraging) -> Result<(), MxError> {
        self._write_and_check(&format!("DAMPING{} {}", channel, value.as_str()))
    }

    /// Set the current limit step size of the output channel.
    pub fn set_current_step_size(&mut self, channel: u8, size: f32) -> Result<(), MxError> {
        self._write_and_check(&format!("DELTAI{} {:.3}", channel, size))
    }

    /// Set the Multi-On action of the output channel.
    pub fn set_multi_on_action(&mut self, channel: u8, action: MultiActionType) -> Result<(), MxError> {
        let action_str = match action {
            MultiActionType::Quick => "QUICK",
            MultiActionType::Never => "NEVER",
            MultiActionType::Delay => "DELAY",
        };
        self._write_and_check(&format!("ONACTION{} {}", channel, action_str))
    }

    /// Set the Multi-On delay, in milliseconds, of the output channel.
    pub fn set_multi_on_delay(&mut self, channel: u8, delay_ms: u16) -> Result<(), MxError> {
        self._write_and_check(&format!("ONDELAY{} {}", channel, delay_ms))
    }

    /// Set the Multi-Off action of the output channel.
    pub fn set_multi_off_action(&mut self, channel: u8, action: MultiActionType) -> Result<(), MxError> {
        let action_str = match action {
            MultiActionType::Quick => "QUICK",
            MultiActionType::Never => "NEVER",
            MultiActionType::Delay => "DELAY",
        };
        self._write_and_check(&format!("OFFACTION{} {}", channel, action_str))
    }

    /// Set the Multi-Off delay, in milliseconds, of the output channel.
    pub fn set_multi_off_delay(&mut self, channel: u8, delay_ms: u16) -> Result<(), MxError> {
        self._write_and_check(&format!("OFFDELAY{} {}", channel, delay_ms))
    }

    /// Set the over-current protection trip point of the output channel.
    pub fn set_over_current_protection(&mut self, channel: u8, enable: bool, value: Option<f32>) -> Result<(), MxError> {
        let command = if enable {
            match value {
                Some(val) => format!("OCP{channel} ON;OCP{channel} {value:.3}", channel=channel, value=val),
                None => return Err(MxError::InvalidParameter("Must specify OCP value if enabling.".to_string())),
            }
        } else {
            format!("OCP{} OFF", channel)
        };
        self._write_and_check(&command)
    }

    /// Set the over-voltage protection trip point of the output channel.
    pub fn set_over_voltage_protection(&mut self, channel: u8, enable: bool, value: Option<f32>) -> Result<(), MxError> {
        let command = if enable {
            match value {
                Some(val) => format!("OVP{channel} ON;OVP{channel} {value:.3}", channel=channel, value=val),
                None => return Err(MxError::InvalidParameter("Must specify OVP value if enabling.".to_string())),
            }
        } else {
            format!("OVP{} OFF", channel)
        };
        self._write_and_check(&command)
    }

    /// Set the output voltage of the output channel.
    pub fn set_voltage(&mut self, channel: u8, value: f32, verify: bool) -> Result<(), MxError> {
        let command = if verify {
            format!("V{}V {:.3}", channel, value)
        } else {
            format!("V{} {:.3}", channel, value)
        };
        self._write_and_check(&command)
    }

    /// Set the output voltage range of the output channel.
    pub fn set_voltage_range(&mut self, channel: u8, index: i32) -> Result<(), MxError> {
        self._write_and_check(&format!("VRANGE{} {}", channel, index))
    }

    /// Set the voltage step size of the output channel.
    pub fn set_voltage_step_size(&mut self, channel: u8, size: f32) -> Result<(), MxError> {
        self._write_and_check(&format!("DELTAV{} {:.3}", channel, size))
    }

    /// Set the voltage tracking mode of the unit.
    pub fn set_voltage_tracking_mode(&mut self, mode: i32) -> Result<(), MxError> {
        self._write_and_check(&format!("CONFIG {}", mode))
    }
}
