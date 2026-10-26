// Preflight only: load the same simulator dependency as simulate-avr.cjs.
const path = require('node:path');
const avr = require(process.env.HELIUM_AVR8JS ||
    path.resolve(__dirname, '../build/toolchain/avr8js/package'));
if (!avr.CPU || !avr.AVRTimer || !avr.avrInstruction || !avr.timer1Config) {
    throw new Error('AVR8js is missing required CPU/Timer1 exports');
}
