use crate::rpc::{Request, Response};
use crate::VirtualMachineInfo;
use paste::paste;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

macro_rules! define_requests {
    ($($name:ident($req:tt, $resp:tt))+) => {
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(tag = "query", rename_all = "snake_case")]
        pub enum AllRequests {
            $($name(Box<paste! { [<$name Request >] }>)),+
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(tag = "answer", rename_all = "snake_case")]
        pub enum AllResponses {
            $($name(Box<paste! { [<$name Response >] }>)),+
        }

        $(
            paste! {
                #[derive(Clone, Debug, Serialize, Deserialize)]
                pub struct [<$name Request>] $req
                #[derive(Clone, Debug, Serialize, Deserialize)]
                pub struct [<$name Response>] $resp

                impl Request for [<$name Request>] {
                    type Response = [<$name Response>];

                    fn into_enum(self) -> AllRequests {
                        AllRequests::$name(Box::new(self))
                    }
                }

                impl Response for [<$name Response>] {
                    fn into_enum(self) -> AllResponses {
                        AllResponses::$name(Box::new(self))
                    }
                }
            }
        )+
    };
}

impl Request for AllRequests {
    type Response = AllResponses;

    fn into_enum(self) -> AllRequests {
        self
    }
}

impl Response for AllResponses {
    fn into_enum(self) -> AllResponses {
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiskPreset {
    pub name: String,
    pub description: String,
}

define_requests! {
    Info({}, {
        pub name: String,
        pub version: String
    })

    List({}, {
        pub items: Vec<VirtualMachineInfo>
    })

    Load({
        pub toml: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub cdroms: Vec<String>,
        #[serde(default)]
        pub save: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub working_directory: Option<String>,
    }, {
        pub info: VirtualMachineInfo,
    })

    Prepare({
        pub name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub cdroms: Vec<String>,
    }, {})

    Start({
        pub name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub cdroms: Vec<String>,
    }, {})

    Stop({
        pub name: String,
    }, {})

    Unload({
        pub name: String,
    }, {})

    Kill({
        pub name: String,
    }, {})

    DiskPresets({}, {
        pub presets: Vec<DiskPreset>
    })
}
