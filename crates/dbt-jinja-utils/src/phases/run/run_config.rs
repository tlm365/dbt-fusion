//! Module for the parse config object to be used during parsing

use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use dbt_common::serde_utils::convert_json_to_map;
use minijinja::{
    arg_utils::ArgParser, listener::RenderingEventListener, value::Object, Error as MinijinjaError,
    ErrorKind as MinijinjaErrorKind, State, Value,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ModelConfig {
    pub config: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelNode {
    pub config: BTreeMap<String, Value>,
}
impl Object for ModelNode {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        if key.as_str().unwrap() == "config" {
            return Some(Value::from_serialize(ModelConfig {
                config: self.config.clone(),
            }));
        }
        self.config.get(key.as_str().unwrap()).cloned()
    }
}

/// A struct that represents a runtime config object to be used during runtime
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// A map of config values to be used during runtime
    pub config: BTreeMap<String, Value>,
}

impl Object for RunConfig {
    /// Get the value of a key from the config
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        if key.as_str().unwrap() == "model" {
            return Some(Value::from_object(ModelNode {
                config: convert_json_to_map(serde_json::to_value(self.config.clone()).unwrap()),
            }));
        }
        self.config.get(key.as_str().unwrap()).cloned()
    }

    fn call_method(
        self: &Arc<Self>,
        _state: &State<'_, '_>,
        name: &str,
        args: &[Value],
        _listener: Rc<dyn RenderingEventListener>,
    ) -> Result<Value, MinijinjaError> {
        match name {
            // At compile time, this will return the value of the config variable if it exists
            // Here, we just return an empty string
            "get" => {
                let mut args = ArgParser::new(args, None);
                let name: String = args.get("name")?;
                let default = args
                    .get_optional::<Value>("default")
                    .unwrap_or(Value::from(None::<Option<String>>));

                match self.config.get(&name) {
                    Some(val) => {
                        if val.is_none() {
                            Ok(default)
                        } else {
                            Ok(val.clone())
                        }
                    }
                    _ => Ok(default),
                }
            }
            // At compile time, this just returns an empty string
            "set" => {
                let mut args = ArgParser::new(args, None);
                let _: String = args.get("name")?;
                Ok(Value::from(""))
            }
            // At compile time, this will throw an error if the config required does not exist
            "require" => {
                let mut args = ArgParser::new(args, None);
                let _: String = args.get("name")?;
                Ok(Value::from(""))
            }
            "persist_relation_docs" => {
                let default_value = Value::from(BTreeMap::<String, Value>::new());
                let persist_docs = match self.config.get("persist_docs") {
                    Some(val) if !val.is_none() => val,
                    _ => &default_value,
                };
                let persist_docs_map = match persist_docs.as_object() {
                    Some(obj) => obj,
                    None => {
                        return Err(MinijinjaError::new(
                            MinijinjaErrorKind::InvalidOperation,
                            "persist_docs must be a dictionary".to_string(),
                        ))
                    }
                };

                Ok(persist_docs_map
                    .get_value(&Value::from("relation"))
                    .unwrap_or(Value::from(false)))
            }
            "persist_column_docs" => {
                let default_value = Value::from(BTreeMap::<String, Value>::new());
                let persist_docs = match self.config.get("persist_docs") {
                    Some(val) if !val.is_none() => val,
                    _ => &default_value,
                };
                let persist_docs_map = match persist_docs.as_object() {
                    Some(obj) => obj,
                    None => {
                        return Err(MinijinjaError::new(
                            MinijinjaErrorKind::InvalidOperation,
                            "persist_docs must be a dictionary".to_string(),
                        ))
                    }
                };

                Ok(persist_docs_map
                    .get_value(&Value::from("columns"))
                    .unwrap_or(Value::from(false)))
            }
            _ => Err(MinijinjaError::new(
                MinijinjaErrorKind::UnknownMethod("RunConfig".to_string(), name.to_string()),
                format!("Unknown method on parse: {}", name),
            )),
        }
    }
}
