//! Test sending messages and receiving commands using the DMA
#![deny(unsafe_code)]
#![deny(warnings)]
#![feature(const_fn)]
#![feature(proc_macro)]
#![no_std]

extern crate cortex_m_rtfm as rtfm;
extern crate f4;
extern crate heapless;

use core::ops::Deref;
use f4::Serial;
use f4::prelude::*;
use f4::dma::{Buffer, Dma1Channel5, Dma1Channel6};
use f4::time::Hertz;
use heapless::Vec;
use rtfm::{app, Threshold};

// Max length of a command to be parsed.
const MAX_CMD_LEN: usize = 10;
// Max length of an output string that can be sent.
const MAX_TX_LEN: usize = 100;
// Max length of input buffer. Since we are parsing for commands, we need to check each received character.
const MAX_RX_LEN: usize = 1;

const BAUD_RATE: Hertz = Hertz(115_200);

app! {
    device: f4::stm32f40x,

    resources: {
        static CMD_BUFFER: Vec<u8, [u8; MAX_CMD_LEN]> = Vec::new([0; MAX_CMD_LEN]);
        static RX_BUFFER: Buffer<[u8; MAX_RX_LEN], Dma1Channel5> = Buffer::new([0; MAX_RX_LEN]);
        static TX_BUFFER: Buffer<[u8; MAX_TX_LEN], Dma1Channel6> = Buffer::new([0; MAX_TX_LEN]);
    },

    tasks: {
        DMA1_STREAM5: {
            path: rx_done,
            priority: 1,
            resources: [CMD_BUFFER, RX_BUFFER, TX_BUFFER, DMA1, USART2],
        },
        DMA1_STREAM6: {
            path: tx_done,
            priority: 2,
            resources: [TX_BUFFER, DMA1],
        },
    },
}

fn init(p: init::Peripherals, r: init::Resources) {
    // There is no need to claim() resources in the init.
    // Start the serial port
    let serial = Serial(p.USART2);
    serial.init(BAUD_RATE.invert(), Some(p.DMA1), p.GPIOA, p.RCC);
    // Send a welcome message. We can only clone slices of equal length, else panic.
    r.TX_BUFFER.borrow_mut()[..15].clone_from_slice(b"Hello, world!\r\n");
    serial.write_all(p.DMA1, r.TX_BUFFER).unwrap();
    // Listen to serial input on the receive DMA
    serial.read_exact(p.DMA1, r.RX_BUFFER).unwrap();
}

fn idle() -> ! {
    loop {
        rtfm::wfi();
    }
}

// Interrupt for serial receive DMA
fn rx_done(t: &mut Threshold, mut r: DMA1_STREAM5::Resources) {
    use rtfm::Resource;

    let mut byte: u8 = 0;
    let mut say_hello = false;

    r.RX_BUFFER.claim(t, |rx, t| {
        // We need to unlock the DMA to use it again in the future
        r.DMA1.claim(t, |dma, _| rx.release(dma).unwrap());
        // Read the single character in the input buffer
        byte = rx.deref().borrow()[0];
        // Echo the character back to the sender.
        // We do not need to use DMA to transmit.
        r.USART2.claim(t, |usart, t| {
            let serial = Serial(&**usart);
            if serial.write(byte).is_err() {
                rtfm::bkpt();
            }
            if byte == b'\r' {
                // If we got carrige return, send new line as well
                if serial.write(b'\n').is_err() {
                    rtfm::bkpt();
                }
            }
            // Get ready to receive again
            r.DMA1
                .claim(t, |dma, _| serial.read_exact(dma, rx).unwrap());
        });
    });
    // Parse the user input
    r.CMD_BUFFER.claim_mut(t, |cmd, _| {
        if byte == b'\r' {
            // End of command
            match &***cmd {
                b"hi" => {
                    say_hello = true;
                }
                _ => {}
            }
            cmd.clear();
        } else {
            if cmd.push(byte).is_err() {
                // Error: buffer full
                // KISS: we just clear the buffer when it gets full
                cmd.clear();
            }
        }
    });
    // If user wrote 'hi' and pressed enter, respond appropriately
    if say_hello {
        r.TX_BUFFER.claim_mut(t, |tx, _| {
            let array = &**tx.deref();
            // We can only clone slices of equal length, else panic
            array.borrow_mut()[..15].clone_from_slice(b"Hello, there!\r\n");
        });
        // Transmit the response
        r.TX_BUFFER.claim(t, |tx, t| {
            r.DMA1.claim(t, |dma, t| {
                r.USART2.claim(t, |usart, _| {
                    let serial = Serial(&**usart);
                    serial.write_all(dma, tx).unwrap();
                });
            });
        });
    }
}

// Interrupt for serial transmit DMA
fn tx_done(t: &mut Threshold, r: DMA1_STREAM6::Resources) {
    use rtfm::Resource;
    // We need to unlock the DMA to use it again in the future
    r.TX_BUFFER.claim(t, |tx, t| {
        r.DMA1.claim(t, |dma, _| tx.release(dma).unwrap());
    });
    // Clear the transmit buffer so we do not retransmit old stuff
    r.TX_BUFFER.claim_mut(t, |tx, _| {
        let array = &**tx.deref();
        array.borrow_mut()[..MAX_TX_LEN].clone_from_slice(&[0; MAX_TX_LEN]);
    });
}