use abstract_sdk::features::AbstractResponse;
use chrono::{DateTime, FixedOffset, LocalResult, NaiveTime, TimeZone};
use cosmwasm_std::{DepsMut, Env, Int64, MessageInfo, Response};

use crate::contract::{App, AppResult};

use crate::error::AppError;
use crate::msg::AppExecuteMsg;
use crate::state::{Meeting, CALENDAR, CONFIG};

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
    }
}

/// Update the configuration of the app
fn request_meeting(
    deps: DepsMut,
    info: MessageInfo,
    app: App,
    env: Env,
    meeting_start_time: Int64,
    meeting_end_time: Int64,
) -> AppResult {
    let config = CONFIG.load(deps.storage)?;

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
    });

    CALENDAR.save(deps.storage, start_of_day_timestamp, &existing_meetings)?;

    Ok(app.tag_response(
        Response::default()
            .add_attribute("meeting_start_time", meeting_start_timestamp.to_string())
            .add_attribute("meeting_end_time", meeting_end_timestamp.to_string()),
        "request_meeting",
    ))
}

fn get_date_time(timezone: FixedOffset, timestamp: Int64) -> AppResult<DateTime<FixedOffset>> {
    if let LocalResult::Single(value) = timezone.timestamp_opt(timestamp.i64(), 0) {
        Ok(value)
    } else {
        Err(AppError::InvalidTime {})
    }
}
