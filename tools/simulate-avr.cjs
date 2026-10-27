// Headless ATmega328P harness. GPIO input levels are independent of PORT latches.
// Usage: node tools/simulate-avr.cjs <build-directory> <input-hex-byte-list>
const fs = require('node:fs');
const path = require('node:path');
const { CPU, avrInstruction, AVRTimer, timer1Config } = require(process.env.HELIUM_AVR8JS ||
    path.resolve(__dirname, '../build/toolchain/avr8js/package'));

const directory = process.argv[2];
const input = (process.argv[3] || '').split(',').filter(Boolean).map(x => parseInt(x, 16));
const flash = new Uint8Array(32768);
let base = 0;
let eof = false;
for (const line of fs.readFileSync(path.join(directory, 'program.hex'), 'utf8').trim().split(/\r?\n/)) {
    if (eof || !/^:(?:[0-9a-f]{2})+$/i.test(line)) throw new Error('Invalid HEX record');
    const bytes = [...Buffer.from(line.slice(1), 'hex')];
    const [length, high, low, type] = bytes;
    if (bytes.length !== length + 5 || (bytes.reduce((a, b) => a + b, 0) & 255)) {
        throw new Error('Invalid HEX length or checksum');
    }
    const data = bytes.slice(4, -1);
    if (type === 0) {
        const address = base + high * 256 + low;
        if (address + length > flash.length) throw new Error('HEX exceeds ATmega328P flash');
        flash.set(data, address);
    } else if (type === 1) eof = true;
    else if (type === 2) base = (data[0] * 256 + data[1]) * 16;
    else if (type === 4) base = (data[0] * 256 + data[1]) * 65536;
    else if (type !== 3 && type !== 5) throw new Error(`Unsupported HEX record ${type}`);
}
if (!eof) throw new Error('Missing HEX EOF');

const symbols = new Map();
for (const line of fs.readFileSync(path.join(directory, 'symbols.txt'), 'utf8').split(/\r?\n/)) {
    const match = line.match(/^([0-9a-f]+)\s+(?:([0-9a-f]+)\s+)?([a-zA-Z])\s+(\S+)$/);
    if (match) symbols.set(match[4], { address: parseInt(match[1], 16), size: parseInt(match[2] || '0', 16) });
}
const stop = symbols.get('_exit') || symbols.get('exit');
if (!stop) throw new Error('Missing avr-libc exit symbol');

const cpu = new CPU(new Uint16Array(flash.buffer), 2048);
const timerCycles = Number(process.env.HELIUM_TIMER_CYCLES || 0);
if (timerCycles) new AVRTimer(cpu, timer1Config);
// Adversarial GPIO regression mode: request a real ISR after each main
// instruction once IRQs are enabled. This is fault scheduling, not timer timing.
const racePort = process.env.HELIUM_GPIO_RACE_PORT;
let inIsr = false;
let runtimeStarted = false;
const gpioRace = { mainWrites: 0, isrWrites: 0, mainChanges: 0, isrChanges: 0 };
const injectedCompare = {
    address: timer1Config.compAInterrupt,
    flagRegister: timer1Config.TIFR, flagMask: timer1Config.OCFA,
    enableRegister: timer1Config.TIMSK, enableMask: timer1Config.OCIEA,
};
const interruptCycles = [];
const interruptAddress = symbols.get('__vector_11')?.address;
let nestedInterrupt = false;
const events = [];
// Optional observation only; the harness never changes interrupt state.
const interruptStates = [];
let reads = 0;
for (const [port, address] of [['B', 0x23], ['C', 0x26], ['D', 0x29]]) {
    cpu.readHooks[address] = () => {
        if (reads === input.length) throw new Error('Unexpected extra physical GPIO read');
        const value = input[reads++];
        events.push(`read:${port}:${value}`);
        return value;
    };
    const checkOwnedBits = (register, value) => {
        if (racePort !== port || !runtimeStarted) return;
        const allowed = inIsr ? 0x0a : 0x05;
        if (((cpu.data[register] ^ value) & ~allowed) !== 0) {
            throw new Error(`GPIO lost update: ${inIsr ? 'ISR' : 'main'} modified another pin at ${register}`);
        }
        gpioRace[inIsr ? 'isrWrites' : 'mainWrites']++;
        if (cpu.data[register] !== value) gpioRace[inIsr ? 'isrChanges' : 'mainChanges']++;
    };
    if (racePort === port) {
        cpu.writeHooks[address + 1] = value => {
            checkOwnedBits(address + 1, value);
            return false;
        };
    }
    cpu.writeHooks[address + 2] = value => {
        checkOwnedBits(address + 2, value);
        events.push(`write:${port}:${value}`);
        if (process.env.HELIUM_TRACE_INTERRUPTS) {
            interruptStates.push((cpu.data[0x5f] & 0x80) !== 0);
        }
        return false;
    };
}

let minimumSP = cpu.SP;
let steps = 0;
const maximumSteps = timerCycles ? Math.max(1000000, timerCycles) : 1000000;
while (cpu.pc * 2 !== stop.address && steps < maximumSteps && (!timerCycles || cpu.cycles < timerCycles)) {
    if (timerCycles && cpu.pc * 2 === interruptAddress) {
        inIsr = true;
        interruptCycles.push(cpu.cycles);
        if (cpu.data[0x5f] & 0x80) nestedInterrupt = true;
    }
    if (racePort && (cpu.data[0x5f] & 0x80)) {
        runtimeStarted = true;
        if (!inIsr) cpu.setInterruptFlag(injectedCompare);
    }
    const returningFromIsr = cpu.progMem[cpu.pc] === 0x9518;
    avrInstruction(cpu);
    cpu.tick();
    if (returningFromIsr) inIsr = false;
    minimumSP = Math.min(minimumSP, cpu.SP);
    steps++;
}
if (steps === maximumSteps) throw new Error('Simulation instruction limit reached');
if (reads !== input.length) throw new Error(`Expected ${input.length} GPIO reads, observed ${reads}`);

const globals = [...symbols].filter(([name]) => /^helium_g\d+$/.test(name))
    .sort(([a], [b]) => Number(a.slice(8)) - Number(b.slice(8)))
    .map(([name, {address, size}]) => {
        const offset = address - 0x800000;
        let value = 0;
        for (let i = 0; i < size; i++) value += cpu.data[offset + i] * 256 ** i;
        return `${name}:${value}`;
    });
const directions = [cpu.data[0x24], cpu.data[0x27], cpu.data[0x2a]];
console.log(JSON.stringify({ globals, events, directions, gpioRace, interruptStates, interruptCycles, nestedInterrupt, cycles: cpu.cycles,
    observedStackBytes: 0x8ff - minimumSP, steps }));
