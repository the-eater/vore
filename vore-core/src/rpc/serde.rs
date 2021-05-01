use crate::rpc::{Command, Request, Answer, AnswerResult, AnswerError, Response};
use std::fmt::{Display, Formatter};
use std::fmt;
use std::error::Error;

#[derive(Debug, Default)]
pub struct CommandCenter {
    id: u64,
}

impl CommandCenter {
    pub fn write_command<R: Request>(&mut self, request: R) -> Result<(u64, String), anyhow::Error> {
        let command = Command {
            id: self.id,
            data: request.into_enum(),
        };

        self.id += 1;

        let mut str = serde_json::to_string(&command)?;
        str.push('\n');
        Ok((command.id, str))
    }

    pub fn write_answer<R: Response>(request: &Command, answer: Result<R, anyhow::Error>) -> Result<String, anyhow::Error> {
        let answer = Answer {
            id: request.id,
            data: match answer {
                Ok(data) => AnswerResult::Ok(data),
                Err(err) => AnswerResult::Error(AnswerError {
                    error: format!("{:?}", err)
                })
            },
        };

        let mut str = serde_json::to_string(&answer)?;
        str.push('\n');
        Ok(str)
    }

    pub fn read_command(request: &str) -> Result<Command, anyhow::Error> {
        serde_json::from_str(request).map_err(From::from)
    }

    pub fn read_answer<R: Request>(answer: &str) -> Result<(u64, R::Response), CommandError> {
        log::debug!("Reading answer: {}", answer);
        let answer_obj: Answer<R::Response> = serde_json::from_str(answer).map_err(|err| CommandError::InternalError(err.into()))?;

        match answer_obj.data {
            AnswerResult::Error(err) => Err(CommandError::AnswerError(answer_obj.id, err)),
            AnswerResult::Ok(data) => Ok((answer_obj.id, data))
        }
    }
}

#[derive(Debug)]
pub enum CommandError {
    AnswerError(u64, AnswerError),
    InternalError(anyhow::Error),
}

impl Display for CommandError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::AnswerError(idx, err) => {
                write!(f, "{}\n(rpc call {})", err.error, idx)
            }
            CommandError::InternalError(err) => err.fmt(f)
        }
    }
}

impl Error for CommandError {}