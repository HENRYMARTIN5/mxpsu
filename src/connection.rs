use crate::error::MxError;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;
use std::sync::Mutex;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub trait Connection: Send + Sync {
    fn write_command(&mut self, command: &str) -> Result<(), MxError>;
    fn read_response(&mut self) -> Result<String, MxError>;
    fn query(&mut self, command: &str) -> Result<String, MxError> {
        self.write_command(command)?;
        self.read_response()
    }
    fn set_timeout(&mut self, duration: Duration) -> Result<(), MxError>;
}

#[cfg(feature = "socket")]
pub struct SocketConnection {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

#[cfg(feature = "socket")]
impl SocketConnection {
    pub fn new(address: &str) -> Result<Self, MxError> {
        let stream = TcpStream::connect(address)?;
        stream.set_read_timeout(Some(DEFAULT_TIMEOUT))?;
        stream.set_write_timeout(Some(DEFAULT_TIMEOUT))?;
        let reader_stream = stream.try_clone()?;
        Ok(SocketConnection {
            stream,
            reader: BufReader::new(reader_stream),
        })
    }
}

#[cfg(feature = "socket")]
impl Connection for SocketConnection {
    fn write_command(&mut self, command: &str) -> Result<(), MxError> {
        let full_command = format!("{}\n", command);
        self.stream.write_all(full_command.as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<String, MxError> {
        let mut response = String::new();
        self.reader.read_line(&mut response)?;
        Ok(response.trim().to_string())
    }

    fn set_timeout(&mut self, duration: Duration) -> Result<(), MxError> {
        self.stream.set_read_timeout(Some(duration))?;
        self.stream.set_write_timeout(Some(duration))?;
        Ok(())
    }
}

#[cfg(feature = "serial")]
pub struct SerialConnection {
    port: Mutex<Box<dyn serialport::SerialPort>>,
}

#[cfg(feature = "serial")]
impl SerialConnection {
    pub fn new(port_name: &str, baud_rate: u32) -> Result<Self, MxError> {
        let port = serialport::new(port_name, baud_rate)
            .timeout(DEFAULT_TIMEOUT)
            .open()?;
        Ok(SerialConnection { port: Mutex::new(port) })
    }
}

#[cfg(feature = "serial")]
impl Connection for SerialConnection {
    fn write_command(&mut self, command: &str) -> Result<(), MxError> {
        let full_command = format!("{}\n", command);
        let mut port_guard = self.port.lock().map_err(|_e| MxError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Serial port mutex poisoned")))?;
        port_guard.write_all(full_command.as_bytes())?;
        port_guard.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<String, MxError> {
        let mut serial_buf: Vec<u8> = Vec::new();
        let mut byte_buf = [0; 1];
        let mut port_guard = self.port.lock().map_err(|_e| MxError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Serial port mutex poisoned")))?;
        loop {
            match port_guard.read(&mut byte_buf) {
                Ok(0) => {
                    // End of stream or timeout if no bytes were read.
                    break;
                }
                Ok(1) => {
                    if byte_buf[0] == b'\n' {
                        break;
                    }
                    if byte_buf[0] != b'\r' { // Ignore CR
                        serial_buf.push(byte_buf[0]);
                    }
                }
                Ok(_) => {
                    // This case should ideally not be reached if reading into a 1-byte buffer.
                    // If it is, it implies more than 1 byte was read into a 1-byte buffer,
                    // which is unexpected. Breaking here is a safe default.
                    break; 
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // Timeout occurred
                    break;
                }
                Err(e) => return Err(MxError::Io(e)),
            }
        }
        String::from_utf8(serial_buf)
            .map(|s| s.trim().to_string())
            .map_err(|e| MxError::Parse(format!("Invalid UTF-8 sequence: {}", e)))
    }

    fn set_timeout(&mut self, duration: Duration) -> Result<(), MxError> {
        let mut port_guard = self.port.lock().map_err(|_e| MxError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Serial port mutex poisoned")))?;
        port_guard.set_timeout(duration)?;
        Ok(())
    }
}
