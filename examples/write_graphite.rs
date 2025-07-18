#![cfg(feature = "serde")]

use collectd_plugin::{
    collectd_plugin, CollectdLoggerBuilder, ConfigItem, Plugin, PluginCapabilities, PluginManager,
    PluginRegistration, Value, ValueList,
};
use log::error;
use log::LevelFilter;
use serde::Deserialize;
use std::borrow::Cow;
use std::error;
use std::io::Write;
use std::net::TcpStream;
use std::ops::Deref;
use std::sync::Mutex;

/// Here is what our collectd config can look like:
///
/// ```
/// LoadPlugin write_graphite_rust
/// <Plugin write_graphite_rust>
///     <Node>
///         Name "localhost.1"
///         Address "127.0.0.1:20003"
///     </Node>
///     <Node>
///         Name "localhost.2"
///         Address "127.0.0.1:20004"
///         Prefix "iamprefix"
///     </Node>
/// </Plugin>
/// ```
#[derive(Deserialize, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
struct GraphiteConfig {
    #[serde(rename = "Node")]
    nodes: Vec<GraphiteNode>,
}

#[derive(Deserialize, Debug, PartialEq, Default)]
#[serde(rename_all = "PascalCase")]
#[serde(deny_unknown_fields)]
struct GraphiteNode {
    name: String,
    address: String,
    prefix: Option<String>,
}

struct GraphitePlugin<W: Write + Send> {
    // We need a mutex as writers aren't thread safe
    writer: Mutex<W>,
    prefix: Option<String>,
}

struct GraphiteManager;

impl PluginManager for GraphiteManager {
    fn name() -> &'static str {
        "write_graphite_rust"
    }

    fn plugins(
        config: Option<&[ConfigItem<'_>]>,
    ) -> Result<PluginRegistration, Box<dyn error::Error>> {
        // Register a logging hook so that any usage of the `log` crate will be forwarded to
        // collectd's logging facilities
        CollectdLoggerBuilder::new()
            .prefix_plugin::<Self>()
            .filter_level(LevelFilter::Info)
            .try_init()
            .expect("really the only thing that should create a logger");

        // Deserialize the collectd configuration into our configuration struct
        let config: GraphiteConfig =
            collectd_plugin::de::from_collectd(config.unwrap_or_default())?;

        let config: Vec<(String, Box<dyn Plugin>)> = config
            .nodes
            .into_iter()
            .map(|x| {
                let plugin = GraphitePlugin {
                    writer: Mutex::new(TcpStream::connect(x.address)?),
                    prefix: x.prefix,
                };
                let bx: Box<dyn Plugin> = Box::new(plugin);
                Ok((x.name, bx))
            })
            .collect::<Result<Vec<_>, Box<dyn error::Error>>>()?;

        Ok(PluginRegistration::Multiple(config))
    }
}

/// If necessary removes any characters from a string that have special meaning in graphite.
fn graphitize(s: &str) -> Cow<'_, str> {
    let needs_modifying = s
        .chars()
        .any(|x| x == '.' || x.is_whitespace() || x.is_control());
    if !needs_modifying {
        Cow::Borrowed(s)
    } else {
        let new_s: String = s
            .chars()
            .map(|x| {
                if x == '.' || x.is_whitespace() || x.is_control() {
                    '-'
                } else {
                    x
                }
            })
            .collect();
        Cow::Owned(new_s)
    }
}

impl<W: Write + Send> GraphitePlugin<W> {
    fn write_value(&self, mut line: String, val: Value, dt: &str) {
        line.push(' ');
        line.push_str(val.to_string().as_str());
        line.push(' ');
        line.push_str(dt);
        line.push('\n');

        // Finally, we get our exclusive lock on the tcp writer and send our data down the pipe. If
        // there is a failure, the proper response would be to try and allocate a new connection or
        // backoff. Instead we log the error.
        let mut w = self.writer.lock().unwrap();
        if let Err(ref e) = w.write(line.as_bytes()) {
            error!("could not write to graphite: {}", e);
        }
    }
}

impl<W: Write + Send> Plugin for GraphitePlugin<W> {
    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::WRITE
    }

    fn write_values(&self, list: ValueList<'_>) -> Result<(), Box<dyn error::Error>> {
        // We use a heap allocated string to construct data to send to graphite. Collectd doesn't
        // use the heap (preferring fixed size arrays). We could get the same behavior using the
        // ArrayString type from the arrayvec crate.
        let mut line = String::new();
        if let Some(ref prefix) = self.prefix {
            line.push_str(prefix.as_str());
            line.push('.');
        }
        line.push_str(graphitize(list.host).deref());
        line.push('.');
        line.push_str(graphitize(list.plugin).deref());

        if let Some(instance) = list.plugin_instance {
            line.push('-');
            line.push_str(graphitize(instance).deref());
        }

        line.push('.');
        line.push_str(graphitize(list.type_).deref());

        if let Some(type_instance) = list.type_instance {
            line.push('-');
            line.push_str(graphitize(type_instance).deref());
        }

        let dt = list.time.timestamp().to_string();

        // If there is only one value in the list we don't have to clone our premade string,
        // instead we can write it directly
        if list.values.len() == 1 {
            self.write_value(line, list.values[0].value, dt.as_str());
        } else {
            for v in list.values {
                let mut nv = line.clone();
                nv.push('.');
                nv.push_str(graphitize(v.name).deref());
                self.write_value(nv, v.value, dt.as_str());
            }
        }

        Ok(())
    }
}

collectd_plugin!(GraphiteManager);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_graphitize() {
        assert_eq!(graphitize("hello").deref(), "hello");
        assert_eq!(graphitize("hello.maty").deref(), "hello-maty");
        assert_eq!(graphitize("foo@bar.com").deref(), "foo@bar-com");
        assert_eq!(graphitize("  test \n ").deref(), "--test---");
    }
}
