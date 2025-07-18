#![cfg(feature = "serde")]

use chrono::Duration;
use collectd_plugin::{
    collectd_log, collectd_log_raw, collectd_plugin, CollectdLoggerBuilder, ConfigItem, LogLevel,
    Plugin, PluginCapabilities, PluginManager, PluginRegistration, ValueList,
};
use log::{info, LevelFilter};
use serde::Deserialize;
use std::{error, fmt::Write};

fn true_default() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct LogWritePlugin {
    #[serde(default = "true_default", rename = "StoreRates")]
    store_rates: bool,
}

impl Drop for LogWritePlugin {
    fn drop(&mut self) {
        info!("yes drop is called");
    }
}

impl PluginManager for LogWritePlugin {
    fn name() -> &'static str {
        "write_logrs"
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

        collectd_log_raw!(LogLevel::Info, b"A raw log with argument: %d\0", 10);
        let line = format!("collectd logging configuration: {:?}", config);
        collectd_log(LogLevel::Info, &line);
        info!("rust logging configuration: {:?}", config);
        let plugin: LogWritePlugin =
            collectd_plugin::de::from_collectd(config.unwrap_or_default())?;
        Ok(PluginRegistration::Single(Box::new(plugin)))
    }
}

impl Plugin for LogWritePlugin {
    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::WRITE | PluginCapabilities::FLUSH
    }

    fn write_values(&self, list: ValueList<'_>) -> Result<(), Box<dyn error::Error>> {
        let values = if self.store_rates {
            list.rates()
        } else {
            Ok(::std::borrow::Cow::Borrowed(&list.values))
        }?;

        // Join values into comma separated output
        let mut out = String::new();
        for (ind, v) in values.iter().enumerate() {
            if ind != 0 {
                out.push_str(", ");
            }
            out.push_str(v.name);
            out.push_str(" - ");
            write!(&mut out, "{}", v.value).expect("string write to succeed");
        }

        info!(
            "plugin_instance: {}, plugin: {}, type: {}, type_instance: {}, host: {}, time: {}, interval: {} seconds, values: {}",
            list.plugin_instance.unwrap_or("<none>"),
            list.plugin,
            list.type_,
            list.type_instance.unwrap_or("<none>"),
            list.host,
            list.time,
            list.interval.num_seconds(),
            out,
        );

        Ok(())
    }

    fn flush(
        &self,
        timeout: Option<Duration>,
        identifier: Option<&str>,
    ) -> Result<(), Box<dyn error::Error>> {
        info!(
            "flushing: timeout: {}, identifier: {}",
            timeout
                .map(|x| format!("{}", x))
                .unwrap_or_else(|| String::from("no timeout")),
            identifier
                .map(|x| x.to_string())
                .unwrap_or_else(|| String::from("no identifier"))
        );
        Ok(())
    }
}

collectd_plugin!(LogWritePlugin);
