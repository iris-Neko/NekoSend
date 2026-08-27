use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
    };
}

string_enum!(Platform { Windows, Android });
string_enum!(PeerRelation {
    Nearby,
    Known,
    OwnDevice
});
string_enum!(Presence { Offline, Online });
string_enum!(ReceivePolicy {
    AutoAccept,
    AskEveryTime
});
string_enum!(ClipboardMode {
    Off,
    SendOnly,
    ReceiveOnly,
    Bidirectional
});
string_enum!(BindingState {
    PendingOutbound,
    PendingInbound,
    Active,
    Rejected,
    Removed
});
string_enum!(LogLevel { Normal, Debug });

string_enum!(ConversationKind { Private, Group });
string_enum!(ConversationState {
    Active,
    Left,
    Disbanded
});
string_enum!(GroupRole { Owner, Member });
string_enum!(GroupMembership {
    Invited,
    Joined,
    Left,
    Removed
});
string_enum!(MessageKind {
    Text,
    File,
    Image,
    Folder,
    ClipboardText,
    ClipboardImage,
    System
});
string_enum!(MessageState {
    Queued,
    Sending,
    PartiallyDelivered,
    Delivered,
    Failed,
    Cancelled
});
string_enum!(DeliveryState {
    Queued,
    Sending,
    Stored,
    Accepted,
    Rejected,
    Completed,
    Failed,
    Cancelled
});

string_enum!(TransferDirection { Send, Receive });
string_enum!(TransferState {
    Queued,
    Offered,
    Accepted,
    Transferring,
    Paused,
    Verifying,
    Completed,
    Failed,
    Cancelled
});
string_enum!(TransferEntryKind { File, Directory });
string_enum!(TransferEntryState {
    Queued,
    Transferring,
    Completed,
    Failed,
    Cancelled
});
string_enum!(TransferFailureReason {
    PeerOffline,
    Rejected,
    SourceChanged,
    NotEnoughSpace,
    ConnectionError,
    InvalidPath,
    PermissionLost,
    Unsupported,
    UserCancelled
});

string_enum!(OutboxState { Pending, InFlight });
string_enum!(InvitationState {
    Pending,
    Accepted,
    Rejected,
    Cancelled
});
string_enum!(GroupState { Active, Disbanded });
string_enum!(OfferDecision { Accept, Reject });
string_enum!(InviteDecision { Accept, Reject });
string_enum!(BindingDecision { Accept, Reject });
string_enum!(TransferFilter {
    All,
    Active,
    Waiting,
    Completed,
    Failed
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_use_authoritative_snake_case_names() {
        let cases = [
            (
                serde_json::to_string(&MessageKind::ClipboardText).unwrap(),
                "\"clipboard_text\"",
            ),
            (
                serde_json::to_string(&MessageState::PartiallyDelivered).unwrap(),
                "\"partially_delivered\"",
            ),
            (
                serde_json::to_string(&TransferFailureReason::NotEnoughSpace).unwrap(),
                "\"not_enough_space\"",
            ),
        ];
        for (actual, expected) in cases {
            assert_eq!(actual, expected);
        }
        assert!(serde_json::from_str::<MessageKind>("\"video\"").is_err());
    }
}
