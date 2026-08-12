//! Shared helper for examples: pick a MeshCore connection (serial, TCP or
//! BLE) from the command line.

use meshcore_rs::MeshCore;

const BAUD_RATE: u32 = 115_200;

pub const USAGE: &str = "\
Usage: exactly one of the following is required
  --serial <port>              connect via serial (e.g. /dev/ttyUSB0)
  --tcp <host:port>             connect via TCP
  --ble <device-name>           connect via BLE
  --help                        print this message";

/// Connection parameters selected from the command line, before actually
/// connecting — kept separate from the I/O so parsing/validation can be
/// unit tested without a real device.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionArgs {
    Serial { port: String, baud_rate: u32 },
    Tcp { host: String, port: u16 },
    Ble { name: String },
}

/// Parses connection parameters from `args` (e.g. `env::args().skip(1)`).
/// Exactly one of `--serial <port>`, `--tcp <host:port>` or
/// `--ble <name>` must be present; anything else (none, more than one,
/// unknown flags, missing values) is an error.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ConnectionArgs, String> {
    let mut selected: Option<ConnectionArgs> = None;
    let mut args = args.into_iter();

    while let Some(flag) = args.next() {
        let parsed = match flag.as_str() {
            "--serial" => {
                let port = args.next().ok_or("--serial requires a <port> argument")?;
                ConnectionArgs::Serial {
                    port,
                    baud_rate: BAUD_RATE,
                }
            }
            "--tcp" => {
                let value = args.next().ok_or("--tcp requires a <host:port> argument")?;
                let (host, port) = value
                    .rsplit_once(':')
                    .ok_or_else(|| format!("--tcp value must be host:port, got {value:?}"))?;
                let port: u16 = port
                    .parse()
                    .map_err(|_| format!("invalid TCP port {port:?}"))?;
                ConnectionArgs::Tcp {
                    host: host.to_string(),
                    port,
                }
            }
            "--ble" => ConnectionArgs::Ble {
                name: args
                    .next()
                    .ok_or("--ble requires a <device-name> argument")?,
            },
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("unrecognized argument {other:?}\n\n{USAGE}")),
        };

        if selected.is_some() {
            return Err(format!(
                "only one of --serial, --tcp or --ble may be given\n\n{USAGE}"
            ));
        }
        selected = Some(parsed);
    }

    selected.ok_or_else(|| format!("one of --serial, --tcp or --ble is required\n\n{USAGE}"))
}

/// Establishes a MeshCore connection per `args`. Errors clearly if the
/// matching crate feature wasn't compiled in.
pub async fn connect(args: &ConnectionArgs) -> Result<MeshCore, Box<dyn std::error::Error>> {
    match args {
        ConnectionArgs::Serial { port, baud_rate } => connect_serial(port, *baud_rate).await,
        ConnectionArgs::Tcp { host, port } => connect_tcp(host, *port).await,
        ConnectionArgs::Ble { name } => connect_ble(name).await,
    }
}

#[cfg(feature = "serial")]
async fn connect_serial(
    port: &str,
    baud_rate: u32,
) -> Result<MeshCore, Box<dyn std::error::Error>> {
    println!("Connecting via serial on {port}...");
    Ok(MeshCore::serial(port, baud_rate).await?)
}
#[cfg(not(feature = "serial"))]
async fn connect_serial(
    _port: &str,
    _baud_rate: u32,
) -> Result<MeshCore, Box<dyn std::error::Error>> {
    Err(
        "the \"serial\" feature is not enabled in this build; rebuild with --features serial"
            .into(),
    )
}

#[cfg(feature = "tcp")]
async fn connect_tcp(host: &str, port: u16) -> Result<MeshCore, Box<dyn std::error::Error>> {
    println!("Connecting via TCP to {host}:{port}...");
    Ok(MeshCore::tcp(host, port).await?)
}
#[cfg(not(feature = "tcp"))]
async fn connect_tcp(_host: &str, _port: u16) -> Result<MeshCore, Box<dyn std::error::Error>> {
    Err("the \"tcp\" feature is not enabled in this build; rebuild with --features tcp".into())
}

#[cfg(feature = "ble")]
async fn connect_ble(name: &str) -> Result<MeshCore, Box<dyn std::error::Error>> {
    println!("Connecting via BLE to {name}...");
    Ok(MeshCore::ble_connect(name).await?)
}
#[cfg(not(feature = "ble"))]
async fn connect_ble(_name: &str) -> Result<MeshCore, Box<dyn std::error::Error>> {
    Err("the \"ble\" feature is not enabled in this build; rebuild with --features ble".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_is_an_error() {
        assert!(parse_args(args(&[])).is_err());
    }

    #[test]
    fn serial_requires_an_explicit_port() {
        assert_eq!(
            parse_args(args(&["--serial", "/dev/ttyACM0"])).unwrap(),
            ConnectionArgs::Serial {
                port: "/dev/ttyACM0".to_string(),
                baud_rate: BAUD_RATE
            }
        );
        assert!(parse_args(args(&["--serial"])).is_err());
    }

    #[test]
    fn tcp_requires_host_and_port() {
        assert_eq!(
            parse_args(args(&["--tcp", "192.168.1.50:5000"])).unwrap(),
            ConnectionArgs::Tcp {
                host: "192.168.1.50".to_string(),
                port: 5000
            }
        );
        assert!(parse_args(args(&["--tcp"])).is_err());
        assert!(parse_args(args(&["--tcp", "no-port-here"])).is_err());
        assert!(parse_args(args(&["--tcp", "host:not-a-number"])).is_err());
    }

    #[test]
    fn ble_requires_a_name() {
        assert_eq!(
            parse_args(args(&["--ble", "MeshCore-1234"])).unwrap(),
            ConnectionArgs::Ble {
                name: "MeshCore-1234".to_string()
            }
        );
        assert!(parse_args(args(&["--ble"])).is_err());
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(parse_args(args(&["--bogus"])).is_err());
    }

    #[test]
    fn rejects_more_than_one_connection_flag() {
        assert!(parse_args(args(&["--serial", "/dev/ttyUSB0", "--ble", "Foo"])).is_err());
    }
}
