use defmt::info;
// use embassy_embedded_hal::shared_bus::asynch::spi;
// use embassy_futures::select::{Either, select};
use embassy_sync::channel;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex};
use embassy_time::Delay;
use esp_hal::{
    Async,
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    peripherals,
    spi::{Mode, master},
    time::Rate,
};
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::mod_params::{ModulationParams, PacketParams, RadioError};
use lora_phy::mod_traits::RadioKind;
use lora_phy::sx126x::{self, Sx126x, Sx1262};
use lora_phy::{LoRa, RxMode};
use panic_rtt_target as _;
use static_cell::StaticCell;

// Data used inside DATA_CHANNEL
struct Data {}
// Statuc channel for communication between data_task and radio_task
// static DATA_CHANNEL: channel::Channel<CriticalSectionRawMutex, Data, 4> = channel::Channel::new();

// From lora_p2p_recieve.rs example:
const LORA_FREQUENCY_IN_HZ: u32 = 868_000_000; // WARNING: Set this appropriately for the region

static SPI_BUS: StaticCell<
    mutex::Mutex<CriticalSectionRawMutex, esp_hal::spi::master::Spi<'static, Async>>,
> = StaticCell::new();

// TODO: Check these gpio pins, example shows GPIO8 nss
struct RadioReqs {
    nss_req: peripherals::GPIO7<'static>,
    sclk: peripherals::GPIO9<'static>,
    mosi: peripherals::GPIO10<'static>,
    miso: peripherals::GPIO11<'static>,

    reset_req: peripherals::GPIO12<'static>,
    busy_req: peripherals::GPIO13<'static>,
    dio1_req: peripherals::GPIO14<'static>,
    spi2: peripherals::SPI2<'static>,
}

#[allow(
    clippy::large_stack_frames,
    reason = "This is the main task, so large size is okay"
)]
#[allow(unused_variables)]
#[embassy_executor::task]
pub async fn radio_task(
    receiver: channel::Receiver<'static, CriticalSectionRawMutex, Data, 4>,
    radio_reqs: RadioReqs,
) -> ! {
    // Static size we can receive
    let mut receiving_buffer = [00u8; 100];
    // Setup radio
    let (mut lora, modulation_params, rx_packet_params) =
        match init_radio(radio_reqs, receiving_buffer.len() as u8).await {
            Ok(tup) => tup,
            Err(err) => {
                // Because this is a task, we cannot return
                info!("Radio error: {}", err);
                loop {
                    // Therefore this loop should signal to the engineer that something is wrong
                    info!("In error state");
                    embassy_time::Timer::after_secs(30).await;
                }
            }
        };
    if let Err(err) = lora
        .prepare_for_rx(RxMode::Continuous, &modulation_params, &rx_packet_params)
        .await
    {
        info!("Radio Error: Preparing for Rx: {}", err);
        loop {
            info!("In error state");
            embassy_time::Timer::after_secs(30).await;
        }
    }
    let expected_msg = b"hello";
    let expected_msg_len = expected_msg.len();
    loop {
        receiving_buffer = [00u8; 100];
        match lora.rx(&rx_packet_params, &mut receiving_buffer).await {
            Ok((received_len, _rx_pkt_status)) => {
                if (received_len == expected_msg_len as u8)
                    && (receiving_buffer[..expected_msg_len] == *expected_msg)
                {
                    info!(
                        "rx successfull: {}",
                        core::str::from_utf8(&receiving_buffer[..received_len as usize]).unwrap()
                    );
                } else {
                    info!("rx unknown packet");
                }
            }
            Err(err) => info!("rx unsuccessfull: {}", err),
        }
        // let race_winner = select(
        //     lora.rx(&rx_packet_params, &mut receiving_buffer),
        //     receiver.receive(),
        // )
        // .await;
        //
        // match race_winner {
        //     // A message appears
        //     Either::First(rx_result) => {
        //         todo!();
        //         // Ok((rec_len, _rx_pkt_status)) => {
        //         //   // Check for successfull, something like CRC, To/From fields
        //         // }
        //         // Err(e) info!("Error in recieve: {:?}", err)
        //     }
        //
        //     // Or a sensor data is ready to be send
        //     Either::Second(data_to_be_sent) => {
        //         todo!();
        //         // lora.tx(...)
        //     }
        // }
    }
}

async fn init_radio(
    radio_reqs: RadioReqs,
    max_length: u8,
) -> Result<(LoRa<impl RadioKind, Delay>, ModulationParams, PacketParams), RadioError> {
    // initialize SPI
    let nss = Output::new(radio_reqs.nss_req, Level::High, OutputConfig::default());

    let reset = Output::new(radio_reqs.reset_req, Level::Low, OutputConfig::default());
    let busy = Input::new(radio_reqs.busy_req, InputConfig::default());
    let dio1 = Input::new(radio_reqs.dio1_req, InputConfig::default());

    let spi = master::Spi::new(
        radio_reqs.spi2,
        master::Config::default()
            .with_frequency(Rate::from_khz(100))
            .with_mode(Mode::_0),
    )
    .expect("SPI init failed")
    .with_sck(radio_reqs.sclk)
    .with_mosi(radio_reqs.mosi)
    .with_miso(radio_reqs.miso)
    .into_async();

    // initialize the static SPI bus
    let spi_bus = SPI_BUS.init(mutex::Mutex::new(spi));
    let spi_device = embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice::new(spi_bus, nss);

    // Create the SX126x(2) configuration
    let sx126x_config = sx126x::Config {
        chip: Sx1262,
        tcxo_ctrl: Some(sx126x::TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: false,
        rx_boost: true,
    };

    // Create radio instance
    let iv =
        GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, None).expect("IV init failed");
    let mut lora = LoRa::new(Sx126x::new(spi_device, iv, sx126x_config), false, Delay)
        .await
        .expect("LoRa radio instance init failed");

    let modulation_params = lora.create_modulation_params(
        lora_phy::mod_params::SpreadingFactor::_10,
        lora_phy::mod_params::Bandwidth::_250KHz,
        lora_phy::mod_params::CodingRate::_4_8,
        LORA_FREQUENCY_IN_HZ,
    )?;

    let rx_packet_params =
        lora.create_rx_packet_params(4, false, max_length, true, false, &modulation_params)?;
    Ok((lora, modulation_params, rx_packet_params))
}
