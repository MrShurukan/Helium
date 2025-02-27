use std::collections::HashMap;
use lazy_static::lazy_static;

pub struct Microcontroller {
    pub include_file_name: &'static str
}

lazy_static! {
    pub static ref MICROCONTROLLERS: HashMap<&'static str, Microcontroller> = {
        let mut m = HashMap::new();

        m.insert("atmega328p", Microcontroller { include_file_name: "m328Pdef.inc" });
        m
    };
}