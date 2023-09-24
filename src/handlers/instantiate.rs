use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};

use crate::contract::{App, AppResult};
use crate::msg::AppInstantiateMsg;
use crate::state::{Config, CONFIG};

pub fn instantiate_handler(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    app: App,
    msg: AppInstantiateMsg,
) -> AppResult {
    let config: Config = Config {
        price_per_minute: msg.price_per_minute,
        utc_offset: msg.utc_offset,
        start_time: msg.start_time,
        end_time: msg.end_time,
    };

    CONFIG.save(deps.storage, &config)?;
    app.admin.set(deps, Some(info.sender))?;

    Ok(Response::new())
}
