// Test-only telemetry; this does not add UART to the Helium language.
#include <avr/io.h>
#include <avr/interrupt.h>
#include <stdint.h>
extern volatile uint16_t helium_g0, helium_g1, helium_g2;
static void put(uint8_t c) {
    while (!(UCSR0A & _BV(UDRE0))) {}
    UDR0=c;
}
static void hex(uint16_t n) {
    for (int8_t shift=12; shift>=0; shift-=4) {
        uint8_t digit=(n>>shift)&15;
        put(digit<10 ? '0'+digit : 'A'+digit-10);
    }
}
void b2a_poll(void) {
    static uint16_t calls;
    static uint8_t initialized;
    if (!initialized) {
        UCSR0A=0; UBRR0H=0; UBRR0L=103; // 9600 baud @ 16 MHz
        UCSR0B=_BV(TXEN0); UCSR0C=_BV(UCSZ01)|_BV(UCSZ00);
        initialized=1;
    }
    if (++calls<4096) return;
    calls=0;
    uint8_t saved=SREG; cli();
    uint16_t ticks=helium_g0, errors=helium_g1, progress=helium_g2;
    SREG=saved;
    put('B'); put('2'); put('A'); put(' ');
    hex(ticks); put(' '); hex(errors); put(' '); hex(progress); put('\r'); put('\n');
}
