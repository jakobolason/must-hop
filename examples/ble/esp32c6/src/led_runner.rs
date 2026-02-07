use embassy_time::{Duration, Timer};
use esp_hal::{gpio::AnyPin, rmt::ChannelCreator};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use panic_rtt_target as _;
use smart_leds::{
    RGB8, SmartLedsWriteAsync, brightness, gamma,
    hsv::{Hsv, hsv2rgb},
};

#[embassy_executor::task]
pub async fn slide_rbg_colors(
    // mut rmt_channel: esp_hal::rmt::Channel<'static, esp_hal::Blocking, esp_hal::rmt::Tx>,
    rmt_channel: ChannelCreator<'static, esp_hal::Async, 0>,
    gpio8: AnyPin<'static>,
) {
    let mut rmt_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];
    let mut led = SmartLedsAdapterAsync::new(rmt_channel, gpio8, &mut rmt_buffer);

    let mut color = Hsv {
        hue: 0,
        sat: 255,
        val: 255,
    };
    let mut data: RGB8;
    let level: u8 = 10;

    loop {
        for hue in 0..=255 {
            color.hue = hue;

            data = hsv2rgb(color);

            led.write(brightness(gamma([data].into_iter()), level))
                .await
                .unwrap();
            Timer::after(Duration::from_millis(100)).await;
        }
    }
}
