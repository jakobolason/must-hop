use bt_hci::{cmd::le::*, controller::*};

pub trait ExtAdvController:
    Controller
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
