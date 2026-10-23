/// Deliberately small target description; timers and other resources come later.
pub struct Microcontroller {
    pub name: &'static str,
    pub avr_dude_name: &'static str,
    pub definitions: &'static str,
}
pub const ATMEGA328P: Microcontroller = Microcontroller {
    name: "atmega328p",
    avr_dude_name: "m328p",
    definitions: include_str!("../../assembly_includes/m328Pdef.inc"),
};
pub fn get(name: &str) -> Option<&'static Microcontroller> {
    match name {
        "atmega328p" => Some(&ATMEGA328P),
        _ => None,
    }
}
impl Microcontroller {
    pub fn has_digital_pin(&self, port: &str, number: usize) -> bool {
        match port {
            "B" | "D" => number < 8,
            "C" => number < 7,
            _ => false,
        }
    }
}
