use abstract_sdk::features::AbstractResponse;
use chrono::{DateTime, FixedOffset, LocalResult, NaiveTime, TimeZone, Timelike};
use cosmwasm_std::{BankMsg, Coin, DepsMut, Env, Int64, MessageInfo, Response, Uint128};
use cw_utils::must_pay;

use crate::contract::{App, AppResult};

use crate::error::AppError;
use crate::msg::AppExecuteMsg;
use crate::state::{Meeting, CALENDAR, CONFIG};

enum StakeAction {
    Return,
    FullSlash,
    PartialSlash { minutes_late: u32 },
}

pub fn execute_handler(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    app: App,
    msg: AppExecuteMsg,
) -> AppResult {
    match msg {
        AppExecuteMsg::RequestMeeting {
            start_time,
            end_time,
        } => request_meeting(deps, info, app, env, start_time, end_time),
        AppExecuteMsg::SlashFullStake {
            day_datetime,
            meeting_index,
        } => handle_stake(
            deps,
            info,
            app,
            env,
            day_datetime,
            meeting_index,
            StakeAction::FullSlash,
        ),
        AppExecuteMsg::SlashPartialStake {
            day_datetime,
            meeting_index,
            minutes_late,
        } => handle_stake(
            deps,
            info,
            app,
            env,
            day_datetime,
            meeting_index,
            StakeAction::PartialSlash { minutes_late },
        ),
        AppExecuteMsg::ReturnStake {
            day_datetime,
            meeting_index,
        } => handle_stake(
            deps,
            info,
            app,
            env,
            day_datetime,
            meeting_index,
            StakeAction::Return,
        ),
    }
}

fn request_meeting(
    deps: DepsMut,
    info: MessageInfo,
    app: App,
    env: Env,
    meeting_start_time: Int64,
    meeting_end_time: Int64,
) -> AppResult {
    let config = CONFIG.load(deps.storage)?;
    let amount_sent = must_pay(&info, &config.denom)?;

    let timezone: FixedOffset = FixedOffset::east_opt(config.utc_offset).unwrap();
    let meeting_start_datetime = get_date_time(timezone, meeting_start_time)?;
    let meeting_start_time: NaiveTime = meeting_start_datetime.time();

    let meeting_end_datetime = get_date_time(timezone, meeting_end_time)?;
    let meeting_end_time: NaiveTime = meeting_end_datetime.time();

    // Check that date falls between the given range.
    let calendar_start_time: NaiveTime = config.start_time.into();
    let calendar_end_time: NaiveTime = config.end_time.into();

    let meeting_start_timestamp = meeting_start_datetime.timestamp();
    let meeting_end_timestamp = meeting_end_datetime.timestamp();

    if meeting_start_datetime.date_naive() != meeting_end_datetime.date_naive() {
        return Err(AppError::StartAndEndTimeNotOnSameDay {});
    }

    if meeting_start_time.second() != 0 || meeting_start_time.nanosecond() != 0 {
        return Err(AppError::StartTimeNotRoundedToNearestMinute {});
    }

    if meeting_end_time.second() != 0 || meeting_end_time.nanosecond() != 0 {
        return Err(AppError::EndTimeNotRoundedToNearestMinute {});
    }

    // Not 100% sure about this typecasting but the same is done in the cosmwasm doc example using
    // chrono so it should be fine.
    if (env.block.time.seconds() as i64) > meeting_start_timestamp {
        return Err(AppError::StartTimeMustBeInFuture {});
    }

    if meeting_start_time >= meeting_end_time {
        return Err(AppError::EndTimeMustBeAfterStartTime {});
    }

    if meeting_start_time < calendar_start_time || meeting_start_time > calendar_end_time {
        return Err(AppError::StartTimeDoesNotFallWithinCalendarBounds {});
    }

    if meeting_end_time < calendar_start_time || meeting_end_time > calendar_end_time {
        return Err(AppError::EndTimeDoesNotFallWithinCalendarBounds {});
    }

    // This number will be positive enforced by previous checks.
    let duration_in_minutes: Uint128 =
        Uint128::new((meeting_end_time - meeting_start_time).num_minutes() as u128);

    let expected_amount = duration_in_minutes * config.price_per_minute;
    if amount_sent != expected_amount {
        return Err(AppError::InvalidStakeAmountSent { expected_amount });
    }

    // Get unix start date of the current day
    let start_of_day_timestamp: i64 = meeting_start_datetime
        .date_naive()
        .and_time(NaiveTime::default())
        .timestamp();

    let mut existing_meetings: Vec<Meeting> = CALENDAR
        .may_load(deps.storage, start_of_day_timestamp)?
        .unwrap_or_default();

    if !existing_meetings.is_empty() {
        //Validate that there are no colisions.
        for meeting in existing_meetings.iter() {
            let start_time_conflicts = meeting.start_time <= meeting_start_timestamp
                && meeting_start_timestamp < meeting.end_time;

            let end_time_conflicts = meeting.start_time < meeting_end_timestamp
                && meeting_end_timestamp <= meeting.end_time;

            if start_time_conflicts || end_time_conflicts {
                return Err(AppError::MeetingConflictExists {});
            }
        }
    }
    existing_meetings.push(Meeting {
        start_time: meeting_start_timestamp,
        end_time: meeting_end_timestamp,
        requester: info.sender,
        amount_staked: amount_sent,
    });

    CALENDAR.save(deps.storage, start_of_day_timestamp, &existing_meetings)?;

    Ok(app.tag_response(
        Response::default()
            .add_attribute("meeting_start_time", meeting_start_timestamp.to_string())
            .add_attribute("meeting_end_time", meeting_end_timestamp.to_string()),
        "request_meeting",
    ))
}

fn handle_stake(
    deps: DepsMut,
    info: MessageInfo,
    app: App,
    env: Env,
    day_datetime: Int64,
    meeting_index: u32,
    stake_action: StakeAction,
) -> AppResult {
    app.admin.assert_admin(deps.as_ref(), &info.sender)?;

    let config = CONFIG.load(deps.storage)?;

    let meetings = CALENDAR.may_load(deps.storage, day_datetime.i64())?;
    if meetings.is_none() {
        return Err(AppError::NoMeetingsAtGivenDayDateTime {});
    }
    let mut meetings = meetings.unwrap();
    if meeting_index as usize >= meetings.len() {
        return Err(AppError::MeetingDoesNotExist {});
    }
    let meeting: &mut Meeting = meetings.get_mut(meeting_index as usize).unwrap();

    if (env.block.time.seconds() as i64) <= meeting.end_time {
        return Err(AppError::MeetingNotFinishedYet {});
    }

    let amount_staked = meeting.amount_staked;
    let requester = meeting.requester.to_string();
    if amount_staked.is_zero() {
        return Err(AppError::StakeAlreadyHandled {});
    }

    meeting.amount_staked = Uint128::zero();

    let response = match stake_action {
        StakeAction::Return => app.tag_response(
            Response::default().add_message(BankMsg::Send {
                to_address: requester,
                amount: vec![Coin::new(amount_staked.into(), config.denom)],
            }),
            "return_stake",
        ),
        StakeAction::FullSlash => app.tag_response(
            Response::default().add_message(BankMsg::Send {
                to_address: app.admin.get(deps.as_ref())?.unwrap().to_string(),
                amount: vec![Coin::new(amount_staked.into(), config.denom)],
            }),
            "full_slash",
        ),
        StakeAction::PartialSlash { minutes_late } => {
            let meeting_duration_in_minutes: i64 = (meeting.end_time - meeting.start_time) / 60;
            let amount_to_slash =
                amount_staked.multiply_ratio(minutes_late, meeting_duration_in_minutes as u128);

            app.tag_response(
                Response::default()
                    .add_message(BankMsg::Send {
                        to_address: requester,
                        amount: vec![Coin::new(
                            (amount_staked - amount_to_slash).into(),
                            config.denom.clone(),
                        )],
                    })
                    .add_message(BankMsg::Send {
                        to_address: app.admin.get(deps.as_ref())?.unwrap().to_string(),
                        amount: vec![Coin::new(amount_to_slash.into(), config.denom)],
                    }),
                "partial_slash",
            )
        }
    };

    CALENDAR.save(deps.storage, day_datetime.i64(), &meetings)?;

    Ok(response)
}

fn get_date_time(timezone: FixedOffset, timestamp: Int64) -> AppResult<DateTime<FixedOffset>> {
    if let LocalResult::Single(value) = timezone.timestamp_opt(timestamp.i64(), 0) {
        Ok(value)
    } else {
        Err(AppError::InvalidTime {})
    }
}
