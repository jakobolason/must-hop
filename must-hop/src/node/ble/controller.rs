use bt_hci::{cmd::le::*, controller::*};

// TODO: Find a better method than this, this is stupid that we have to write it up twice, and it
// can't just be inferred somehow
pub trait ExtAdvController:
    Controller
    + ControllerCmdSync<LeSetScanParams>
    + ControllerCmdSync<LeSetExtScanParams>
    + ControllerCmdSync<LeSetPeriodicAdvParams>
    + ControllerCmdSync<LeSetExtAdvParams>
    + ControllerCmdSync<LeSetAdvSetRandomAddr>
    + ControllerCmdSync<LeClearAdvSets>
    + ControllerCmdSync<LeSetPeriodicAdvEnable>
    + for<'k> ControllerCmdSync<LeSetExtAdvData<'k>>
    + for<'t> ControllerCmdSync<LeSetPeriodicAdvData<'t>>
    + for<'t> ControllerCmdSync<LeSetExtAdvEnable<'t>>
    + ControllerCmdSync<LeReadNumberOfSupportedAdvSets>
    + for<'t> ControllerCmdSync<LeSetExtScanResponseData<'t>>
{
}

impl<T> ExtAdvController for T where
    T: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdSync<LeSetExtScanParams>
        + ControllerCmdSync<LeSetPeriodicAdvParams>
        + ControllerCmdSync<LeSetExtAdvParams>
        + ControllerCmdSync<LeSetAdvSetRandomAddr>
        + ControllerCmdSync<LeClearAdvSets>
        + ControllerCmdSync<LeSetPeriodicAdvEnable>
        + for<'k> ControllerCmdSync<LeSetExtAdvData<'k>>
        + for<'t> ControllerCmdSync<LeSetPeriodicAdvData<'t>>
        + for<'t> ControllerCmdSync<LeSetExtAdvEnable<'t>>
        + ControllerCmdSync<LeReadNumberOfSupportedAdvSets>
        + for<'t> ControllerCmdSync<LeSetExtScanResponseData<'t>>
{
}
