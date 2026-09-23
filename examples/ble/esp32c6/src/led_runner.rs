use esp_hal::{delay::Delay, gpio::AnyPin, peripherals::RMT, rmt::Rmt, time::Rate};
use esp_hal_smartled::{RmtSmartLeds, buffer_size, color_order};
use panic_rtt_target as _;
use smart_leds::SmartLedsWrite;
use smart_leds::{
    RGB8, brightness, gamma,
    hsv::{Hsv, hsv2rgb},
};

#[embassy_executor::task]
pub async fn slide_rbg_colors(rmt: RMT<'static>, gpio8: AnyPin<'static>) {
    type LedColor = RGB8;
    const COUNT: usize = 60;
    let freq = Rate::from_mhz(80);
    let mut led = {
        let rmt = Rmt::new(rmt, freq).expect("Failed to initialize RMT0");
        // Configure color order and timing implementation as needed.
        RmtSmartLeds::<{  buffer_size::<LedColor>(COUNT) }, _, LedColor, color_order::Grb>::new_with_memsize(
            esp_hal_smartled::WS2812_TIMING.with_reset_us(305),
            rmt.channel0,
            gpio8,
            2,
            freq
        ).unwrap()
    };

    let delay = Delay::new();

    let mut color = Hsv {
        hue: 0,
        sat: 255,
        val: 255,
    };
    let mut data = [hsv2rgb(color); COUNT];

    let mut start_hue = 0;
    loop {
        start_hue += 1;
        // Iterate over the rainbow!
        for hue in 0..(COUNT as u8) {
            color.hue = hue.wrapping_add(start_hue);
            // Convert from the HSV color space (where we can easily transition from one
            // color to the other) to the RGB color space that we can then send to the LED
            data[hue as usize] = hsv2rgb(color);
        }
        // When sending to the LED, we do a gamma correction first (see smart_leds
        // documentation for details) and then limit the brightness to 10 out of 255 so
        // that the output it's not too bright.
        led.write(brightness(gamma(data.iter().cloned()), 10))
            .unwrap();
        delay.delay_millis(5);
    }
}
