// Generated from Rust serde types. Run the protocol-generate example to update.
import { decode } from '../decoder.ts';
export type JsonValue = null | boolean | number | string | ReadonlyArray<JsonValue> | { readonly [key: string]: JsonValue };
export type AccountUsage = { readonly "accountId": string; readonly "credits"?: (CreditsReading | null); readonly "fiveHour"?: (AllowanceReading | null); readonly "provider": string; readonly "weekly"?: (AllowanceReading | null); };
export type AddProject = { readonly "name"?: string | null; readonly "path": string; readonly "projectId": string; };
export type Agent = "claude" | "codex" | "pi";
export type AgentSessionChangedEvent = { readonly "agent_session_id": string; readonly "previous"?: string | null; };
export type AllowanceReading = { readonly "observedAt": number; readonly "resetsAt"?: number | null; readonly "usedPercent": number; };
export type AnswerQuestion = { readonly "answer": string; readonly "questionId": string; readonly "sessionId": string; };
export type ArchiveChangedEvent = { readonly "archived": boolean; };
export type AttachmentInfo = { readonly "contentType": string; readonly "id": string; readonly "name": string; readonly "size": number; };
export type BackgroundJob = { readonly "agentKey"?: string | null; readonly "command": string; readonly "id": string; };
export type ChatHistory = { readonly "after"?: (Sequence | null); readonly "before"?: (Sequence | null); readonly "sessionId": string; };
export type ChatHistoryPage = { readonly "context": ReadonlyArray<StoredEvent>; readonly "cursor": Sequence; readonly "events": ReadonlyArray<StoredEvent>; readonly "hasMore": boolean; readonly "partialId"?: string | null; readonly "sessionId": string; readonly "status": SessionStatus; };
export type ChatPresence = { readonly "clientId": string; readonly "reloadCapable"?: boolean; readonly "sessionId"?: string | null; };
export type ChatPresenceReply = { readonly "reloadId"?: string | null; };
export type CommandResponse = { readonly "status": SessionStatus; };
export type ContextUsage = { readonly "usedTokens": number; readonly "windowTokens": number; };
export type CreateSession = { readonly "effort"?: string | null; readonly "model": string; readonly "name"?: string; readonly "projectId": string; readonly "sessionId": string; };
export type CreditsReading = { readonly "balance"?: number | null; readonly "observedAt": number; };
export type DeliveryChangedEvent = { readonly "delivery_state": DeliveryState; readonly "expired_from"?: (ExpiredFrom | null); readonly "retry_count"?: number | null; readonly "uuid": string; };
export type DeliveryRejectedEvent = { readonly "retry_available": boolean; readonly "uuid": string; };
export type DeliveryState = ("queued" | "dispatching" | "started" | "rejected" | "uncertain" | "expired");
export type DirectoryInfo = { readonly "name": string; readonly "path": string; };
export type DirectoryListing = { readonly "directories": ReadonlyArray<DirectoryInfo>; readonly "parent"?: string | null; readonly "path": string; };
export type DiscussQuestionLater = { readonly "questionId": string; readonly "sessionId": string; };
export type DriverErrorEvent = { readonly "message": string; };
export type Empty = Readonly<Record<string, never>>;
export type ErrorInfo = { readonly "code": string; readonly "message": string; };
export type EventPage = { readonly "cursor": Sequence; readonly "events": ReadonlyArray<StoredEvent>; readonly "hasMore": boolean; readonly "sessionId": string; readonly "status": SessionStatus; };
export type ExpiredFrom = ("queued" | "uncertain");
export type JobSnapshot = { readonly "available": boolean; readonly "tasks": ReadonlyArray<BackgroundJob>; };
export type ListDirectories = { readonly "path"?: string | null; };
export type ListSessionsResponse = { readonly "cursor"?: Sequence; readonly "models": ReadonlyArray<ModelOption>; readonly "projects": ReadonlyArray<ProjectInfo>; readonly "sessions": ReadonlyArray<SessionInfo>; };
export type MessageAppendedEvent = { readonly "attachments"?: ReadonlyArray<AttachmentInfo>; readonly "content": string; readonly "delivery_state": DeliveryState; readonly "expired_from"?: (ExpiredFrom | null); readonly "role": Role; readonly "sender_session_id"?: string | null; readonly "uuid": string; };
export type ModelEffortChangedEvent = { readonly "effort"?: string | null; readonly "model": string; };
export type ModelOption = { readonly "agent": Agent; readonly "efforts": ReadonlyArray<string>; readonly "label": string; readonly "model": string; };
export type ProjectInfo = { readonly "createdAt": number; readonly "name": string; readonly "path": string; readonly "projectId": string; };
export type PushConfigReply = { readonly "publicKey"?: string | null; };
export type QuestionInfo = { readonly "answer"?: string | null; readonly "answeredAt"?: number | null; readonly "choices": ReadonlyArray<string>; readonly "createdAt": number; readonly "id": string; readonly "question": string; readonly "sessionId": string; readonly "state": QuestionState; };
export type QuestionState = "pending" | "answered" | "discuss_later";
export type QuestionsReply = { readonly "pendingCount": number; readonly "questions": ReadonlyArray<QuestionInfo>; };
export type RemoveProject = { readonly "projectId": string; };
export type RenameProject = { readonly "name": string; readonly "projectId": string; };
export type RenameSession = { readonly "name": string; readonly "sessionId": string; };
export type ReorderChats = { readonly "projectId": string; readonly "sessionIds": ReadonlyArray<string>; };
export type ReorderProjects = { readonly "projectIds": ReadonlyArray<string>; };
export type RetryMessage = { readonly "messageId": string; readonly "sessionId": string; };
export type Role = "user" | "assistant" | "system";
export type SecretReply = { readonly "message": string; readonly "status": string; };
export type SecretRequestedEvent = { readonly "expires_at": number; readonly "file": string; readonly "key": ReadonlyArray<string>; readonly "request_id": string; };
export type SecretResolvedEvent = { readonly "request_id": string; readonly "status": string; };
export type SendMessage = { readonly "attachments"?: ReadonlyArray<string>; readonly "content": string; readonly "messageId": string; readonly "sessionId": string; };
export type SendMessageResponse = { readonly "accepted": boolean; readonly "messageId": string; readonly "status": SessionStatus; };
export type Sequence = number;
export type SessionCreatedEvent = { readonly "id": string; readonly "parent_session_id"?: string | null; readonly "role"?: string | null; readonly "root_session_id"?: string | null; readonly "working_directory": string; };
export type SessionInfo = { readonly "agent": Agent; readonly "agentSessionId"?: string | null; readonly "createdAt": number; readonly "name": string; readonly "parentSessionId"?: string | null; readonly "projectId": string; readonly "role"?: string | null; readonly "rootSessionId"?: string | null; readonly "sessionId": string; readonly "status": SessionStatus; readonly "subagents"?: (SubagentSummary | null); readonly "workingDirectory": string; };
export type SessionListEvent = ({ readonly "createdAt": number; readonly "kind": "deleted"; readonly "payload": Empty; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "created"; readonly "payload": SessionCreatedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "renamed"; readonly "payload": SessionRenamedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "archived"; readonly "payload": ArchiveChangedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "state_summary"; readonly "payload": SessionStatus; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "subagents_changed"; readonly "payload": Empty; readonly "seq": Sequence; readonly "sessionId": string; });
export type SessionRef = { readonly "sessionId": string; };
export type SessionRenamedEvent = { readonly "name": string; };
export type SessionStatus = { readonly "accountKey"?: string | null; readonly "accountUsage"?: (AccountUsage | null); readonly "archived": boolean; readonly "dispatching": number; readonly "effort"?: string | null; readonly "heldReason"?: string | null; readonly "jobs"?: (JobSnapshot | null); readonly "model": string; readonly "promptSuggestion"?: string | null; readonly "queueHeld": boolean; readonly "queued": number; readonly "rejected": number; readonly "running": boolean; readonly "selectionPending": boolean; readonly "stopping": boolean; readonly "turnActive": boolean; readonly "uncertain": number; readonly "usage"?: (UsageSnapshot | null); };
export type SetModel = { readonly "effort"?: string | null; readonly "model": string; readonly "sessionId": string; };
export type StoredEvent = ({ readonly "createdAt": number; readonly "kind": "session_renamed"; readonly "payload": SessionRenamedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "session_created"; readonly "payload": SessionCreatedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "message_appended"; readonly "payload": MessageAppendedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "delivery_changed"; readonly "payload": DeliveryChangedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "agent_session_changed"; readonly "payload": AgentSessionChangedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "archive_changed"; readonly "payload": ArchiveChangedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "model_effort_changed"; readonly "payload": ModelEffortChangedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "delivery_rejected"; readonly "payload": DeliveryRejectedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "agent_event"; readonly "payload": JsonValue; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "driver_error"; readonly "payload": DriverErrorEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "model_error"; readonly "payload": DriverErrorEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "state_summary"; readonly "payload": SessionStatus; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "session_stopping"; readonly "payload": Empty; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "session_stopped"; readonly "payload": Empty; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "delivery_timeout"; readonly "payload": Empty; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "stop_timeout"; readonly "payload": Empty; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "questions_changed"; readonly "payload": JsonValue; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "secret_requested"; readonly "payload": SecretRequestedEvent; readonly "seq": Sequence; readonly "sessionId": string; } | { readonly "createdAt": number; readonly "kind": "secret_resolved"; readonly "payload": SecretResolvedEvent; readonly "seq": Sequence; readonly "sessionId": string; });
export type SubagentHistory = { readonly "agentKey": string; readonly "before"?: (Sequence | null); readonly "sessionId": string; };
export type SubagentHistoryPage = { readonly "agentKey": string; readonly "cursor": Sequence; readonly "events": ReadonlyArray<StoredEvent>; readonly "hasMore": boolean; readonly "sessionId": string; };
export type SubagentSummary = { readonly "failures": number; readonly "open": number; readonly "questions": number; readonly "reportFailures": number; };
export type SubmitSecret = { readonly "cancel"?: boolean | null; readonly "requestId": string; readonly "value"?: string | null; };
export type SubscribePush = { readonly "auth": string; readonly "endpoint": string; readonly "p256dh": string; };
export type UnsubscribePush = { readonly "endpoint": string; };
export type UsageSnapshot = { readonly "context"?: (ContextUsage | null); readonly "contextUpdatedAt"?: number | null; };
export const contractSchema = {
  "$defs": {
    "AccountUsage": {
      "description": "Allowance readings for one provider account. Context stays on the chat snapshot.",
      "properties": {
        "accountId": {
          "type": "string"
        },
        "credits": {
          "anyOf": [
            {
              "$ref": "#/$defs/CreditsReading"
            },
            {
              "type": "null"
            }
          ]
        },
        "fiveHour": {
          "anyOf": [
            {
              "$ref": "#/$defs/AllowanceReading"
            },
            {
              "type": "null"
            }
          ]
        },
        "provider": {
          "type": "string"
        },
        "weekly": {
          "anyOf": [
            {
              "$ref": "#/$defs/AllowanceReading"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "provider",
        "accountId"
      ],
      "type": "object"
    },
    "AddProject": {
      "additionalProperties": false,
      "properties": {
        "name": {
          "description": "Shown in the sidebar; defaults to the folder's name.",
          "maxLength": 200,
          "minLength": 1,
          "type": [
            "string",
            "null"
          ]
        },
        "path": {
          "description": "Absolute path of a folder on this machine.",
          "maxLength": 4096,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        },
        "projectId": {
          "description": "Client-generated, so a retry cannot add the folder twice.",
          "type": "string"
        }
      },
      "required": [
        "projectId",
        "path"
      ],
      "type": "object"
    },
    "Agent": {
      "description": "The agent that runs a chat. A chat's model decides it (ADR 0023).",
      "enum": [
        "claude",
        "codex",
        "pi"
      ],
      "type": "string"
    },
    "AgentSessionChangedEvent": {
      "additionalProperties": false,
      "properties": {
        "agent_session_id": {
          "type": "string"
        },
        "previous": {
          "description": "The conversation this one replaced; null for the first. A value means `/clear`\nstarted a new conversation in the same chat.",
          "type": [
            "string",
            "null"
          ]
        }
      },
      "required": [
        "agent_session_id"
      ],
      "type": "object"
    },
    "AllowanceReading": {
      "description": "One provider allowance reading, tied to the account that reported it.",
      "properties": {
        "observedAt": {
          "description": "Unix milliseconds when this reading was observed.",
          "format": "int64",
          "type": "integer"
        },
        "resetsAt": {
          "description": "Unix seconds, when the provider reports a reset time.",
          "format": "int64",
          "type": [
            "integer",
            "null"
          ]
        },
        "usedPercent": {
          "format": "double",
          "type": "number"
        }
      },
      "required": [
        "usedPercent",
        "observedAt"
      ],
      "type": "object"
    },
    "AnswerQuestion": {
      "additionalProperties": false,
      "properties": {
        "answer": {
          "type": "string"
        },
        "questionId": {
          "type": "string"
        },
        "sessionId": {
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "questionId",
        "answer"
      ],
      "type": "object"
    },
    "ArchiveChangedEvent": {
      "additionalProperties": false,
      "properties": {
        "archived": {
          "type": "boolean"
        }
      },
      "required": [
        "archived"
      ],
      "type": "object"
    },
    "AttachmentInfo": {
      "additionalProperties": false,
      "description": "An uploaded file as shown to the owner. Also the reply to an upload.",
      "properties": {
        "contentType": {
          "maxLength": 255,
          "minLength": 1,
          "type": "string"
        },
        "id": {
          "pattern": "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
          "type": "string"
        },
        "name": {
          "maxLength": 255,
          "minLength": 1,
          "type": "string"
        },
        "size": {
          "format": "uint64",
          "maximum": 52428800,
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "id",
        "name",
        "contentType",
        "size"
      ],
      "type": "object"
    },
    "BackgroundJob": {
      "additionalProperties": false,
      "properties": {
        "agentKey": {
          "description": "None identifies the parent chat; otherwise the provider's subagent key.",
          "type": [
            "string",
            "null"
          ]
        },
        "command": {
          "type": "string"
        },
        "id": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "command"
      ],
      "type": "object"
    },
    "ChatHistory": {
      "additionalProperties": false,
      "description": "Read a window of displayable chat history. Omit both cursors for the latest page.",
      "properties": {
        "after": {
          "anyOf": [
            {
              "$ref": "#/$defs/Sequence"
            },
            {
              "type": "null"
            }
          ]
        },
        "before": {
          "anyOf": [
            {
              "$ref": "#/$defs/Sequence"
            },
            {
              "type": "null"
            }
          ]
        },
        "sessionId": {
          "type": "string"
        }
      },
      "required": [
        "sessionId"
      ],
      "type": "object"
    },
    "ChatHistoryPage": {
      "additionalProperties": false,
      "properties": {
        "context": {
          "items": {
            "$ref": "#/$defs/StoredEvent"
          },
          "type": "array"
        },
        "cursor": {
          "$ref": "#/$defs/Sequence"
        },
        "events": {
          "items": {
            "$ref": "#/$defs/StoredEvent"
          },
          "type": "array"
        },
        "hasMore": {
          "type": "boolean"
        },
        "partialId": {
          "type": [
            "string",
            "null"
          ]
        },
        "sessionId": {
          "type": "string"
        },
        "status": {
          "$ref": "#/$defs/SessionStatus"
        }
      },
      "required": [
        "sessionId",
        "status",
        "cursor",
        "events",
        "context",
        "hasMore"
      ],
      "type": "object"
    },
    "ChatPresence": {
      "additionalProperties": false,
      "properties": {
        "clientId": {
          "type": "string"
        },
        "reloadCapable": {
          "default": false,
          "description": "This page understands targeted reload replies.",
          "type": "boolean"
        },
        "sessionId": {
          "type": [
            "string",
            "null"
          ]
        }
      },
      "required": [
        "clientId"
      ],
      "type": "object"
    },
    "ChatPresenceReply": {
      "additionalProperties": false,
      "properties": {
        "reloadId": {
          "type": [
            "string",
            "null"
          ]
        }
      },
      "type": "object"
    },
    "CommandResponse": {
      "additionalProperties": false,
      "properties": {
        "status": {
          "$ref": "#/$defs/SessionStatus"
        }
      },
      "required": [
        "status"
      ],
      "type": "object"
    },
    "ContextUsage": {
      "properties": {
        "usedTokens": {
          "format": "uint64",
          "minimum": 0,
          "type": "integer"
        },
        "windowTokens": {
          "format": "uint64",
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "usedTokens",
        "windowTokens"
      ],
      "type": "object"
    },
    "CreateSession": {
      "additionalProperties": false,
      "properties": {
        "effort": {
          "type": [
            "string",
            "null"
          ]
        },
        "model": {
          "description": "Must be a model in the catalog; the model decides the agent.",
          "type": "string"
        },
        "name": {
          "default": "",
          "maxLength": 200,
          "type": "string"
        },
        "projectId": {
          "description": "The project the chat belongs to and works in.",
          "maxLength": 128,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        },
        "sessionId": {
          "maxLength": 128,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "projectId",
        "model"
      ],
      "type": "object"
    },
    "CreditsReading": {
      "description": "A Codex credit balance reading. A missing balance means no numeric balance was reported,\nincluding unlimited accounts.",
      "properties": {
        "balance": {
          "format": "double",
          "type": [
            "number",
            "null"
          ]
        },
        "observedAt": {
          "description": "Unix milliseconds when this reading was observed.",
          "format": "int64",
          "type": "integer"
        }
      },
      "required": [
        "observedAt"
      ],
      "type": "object"
    },
    "DeliveryChangedEvent": {
      "additionalProperties": false,
      "properties": {
        "delivery_state": {
          "$ref": "#/$defs/DeliveryState"
        },
        "expired_from": {
          "anyOf": [
            {
              "$ref": "#/$defs/ExpiredFrom"
            },
            {
              "type": "null"
            }
          ]
        },
        "retry_count": {
          "format": "uint8",
          "maximum": 255,
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ]
        },
        "uuid": {
          "type": "string"
        }
      },
      "required": [
        "uuid",
        "delivery_state"
      ],
      "type": "object"
    },
    "DeliveryRejectedEvent": {
      "additionalProperties": false,
      "properties": {
        "retry_available": {
          "type": "boolean"
        },
        "uuid": {
          "type": "string"
        }
      },
      "required": [
        "uuid",
        "retry_available"
      ],
      "type": "object"
    },
    "DeliveryState": {
      "oneOf": [
        {
          "enum": [
            "queued",
            "dispatching",
            "started",
            "rejected",
            "uncertain"
          ],
          "type": "string"
        },
        {
          "const": "expired",
          "description": "Waited past QUEUED_MESSAGE_MAX_AGE and was never sent.",
          "type": "string"
        }
      ]
    },
    "DirectoryInfo": {
      "additionalProperties": false,
      "properties": {
        "name": {
          "type": "string"
        },
        "path": {
          "type": "string"
        }
      },
      "required": [
        "name",
        "path"
      ],
      "type": "object"
    },
    "DirectoryListing": {
      "additionalProperties": false,
      "properties": {
        "directories": {
          "items": {
            "$ref": "#/$defs/DirectoryInfo"
          },
          "type": "array"
        },
        "parent": {
          "type": [
            "string",
            "null"
          ]
        },
        "path": {
          "type": "string"
        }
      },
      "required": [
        "path",
        "directories"
      ],
      "type": "object"
    },
    "DiscussQuestionLater": {
      "additionalProperties": false,
      "properties": {
        "questionId": {
          "type": "string"
        },
        "sessionId": {
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "questionId"
      ],
      "type": "object"
    },
    "DriverErrorEvent": {
      "additionalProperties": false,
      "properties": {
        "message": {
          "type": "string"
        }
      },
      "required": [
        "message"
      ],
      "type": "object"
    },
    "Empty": {
      "additionalProperties": false,
      "type": "object"
    },
    "ErrorInfo": {
      "additionalProperties": false,
      "properties": {
        "code": {
          "type": "string"
        },
        "message": {
          "type": "string"
        }
      },
      "required": [
        "code",
        "message"
      ],
      "type": "object"
    },
    "EventPage": {
      "additionalProperties": false,
      "properties": {
        "cursor": {
          "$ref": "#/$defs/Sequence"
        },
        "events": {
          "items": {
            "$ref": "#/$defs/StoredEvent"
          },
          "type": "array"
        },
        "hasMore": {
          "type": "boolean"
        },
        "sessionId": {
          "type": "string"
        },
        "status": {
          "$ref": "#/$defs/SessionStatus"
        }
      },
      "required": [
        "sessionId",
        "cursor",
        "events",
        "status",
        "hasMore"
      ],
      "type": "object"
    },
    "ExpiredFrom": {
      "description": "The state an expired message was in when it expired.",
      "oneOf": [
        {
          "enum": [
            "queued"
          ],
          "type": "string"
        },
        {
          "const": "uncertain",
          "description": "The agent never confirmed it, so it may already have been received.",
          "type": "string"
        }
      ]
    },
    "JobSnapshot": {
      "additionalProperties": false,
      "properties": {
        "available": {
          "description": "False retains the last known jobs but cannot establish that restart is safe.",
          "type": "boolean"
        },
        "tasks": {
          "items": {
            "$ref": "#/$defs/BackgroundJob"
          },
          "type": "array"
        }
      },
      "required": [
        "tasks",
        "available"
      ],
      "type": "object"
    },
    "ListDirectories": {
      "additionalProperties": false,
      "description": "Browse folders on the server. Omit path to start at the server user's home.",
      "properties": {
        "path": {
          "type": [
            "string",
            "null"
          ]
        }
      },
      "type": "object"
    },
    "ListSessionsResponse": {
      "additionalProperties": false,
      "properties": {
        "cursor": {
          "$ref": "#/$defs/Sequence",
          "default": 0,
          "description": "Resume the global chat-list stream after this snapshot."
        },
        "models": {
          "items": {
            "$ref": "#/$defs/ModelOption"
          },
          "type": "array"
        },
        "projects": {
          "items": {
            "$ref": "#/$defs/ProjectInfo"
          },
          "type": "array"
        },
        "sessions": {
          "items": {
            "$ref": "#/$defs/SessionInfo"
          },
          "type": "array"
        }
      },
      "required": [
        "models",
        "projects",
        "sessions"
      ],
      "type": "object"
    },
    "MessageAppendedEvent": {
      "additionalProperties": false,
      "properties": {
        "attachments": {
          "items": {
            "$ref": "#/$defs/AttachmentInfo"
          },
          "type": "array"
        },
        "content": {
          "type": "string"
        },
        "delivery_state": {
          "$ref": "#/$defs/DeliveryState"
        },
        "expired_from": {
          "anyOf": [
            {
              "$ref": "#/$defs/ExpiredFrom"
            },
            {
              "type": "null"
            }
          ]
        },
        "role": {
          "$ref": "#/$defs/Role"
        },
        "sender_session_id": {
          "type": [
            "string",
            "null"
          ]
        },
        "uuid": {
          "type": "string"
        }
      },
      "required": [
        "uuid",
        "role",
        "content",
        "delivery_state"
      ],
      "type": "object"
    },
    "ModelEffortChangedEvent": {
      "additionalProperties": false,
      "properties": {
        "effort": {
          "type": [
            "string",
            "null"
          ]
        },
        "model": {
          "type": "string"
        }
      },
      "required": [
        "model"
      ],
      "type": "object"
    },
    "ModelOption": {
      "additionalProperties": false,
      "description": "One model a chat can use, from the catalog in the tuning crate.",
      "properties": {
        "agent": {
          "$ref": "#/$defs/Agent"
        },
        "efforts": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "label": {
          "description": "Short name for display; `model` stays the identifier to send.",
          "type": "string"
        },
        "model": {
          "type": "string"
        }
      },
      "required": [
        "model",
        "label",
        "agent",
        "efforts"
      ],
      "type": "object"
    },
    "ProjectInfo": {
      "additionalProperties": false,
      "description": "A folder chats work in.",
      "properties": {
        "createdAt": {
          "format": "int64",
          "type": "integer"
        },
        "name": {
          "type": "string"
        },
        "path": {
          "type": "string"
        },
        "projectId": {
          "type": "string"
        }
      },
      "required": [
        "projectId",
        "path",
        "name",
        "createdAt"
      ],
      "type": "object"
    },
    "PushConfigReply": {
      "additionalProperties": false,
      "properties": {
        "publicKey": {
          "type": [
            "string",
            "null"
          ]
        }
      },
      "type": "object"
    },
    "QuestionInfo": {
      "additionalProperties": false,
      "properties": {
        "answer": {
          "type": [
            "string",
            "null"
          ]
        },
        "answeredAt": {
          "format": "int64",
          "type": [
            "integer",
            "null"
          ]
        },
        "choices": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "createdAt": {
          "format": "int64",
          "type": "integer"
        },
        "id": {
          "type": "string"
        },
        "question": {
          "type": "string"
        },
        "sessionId": {
          "type": "string"
        },
        "state": {
          "$ref": "#/$defs/QuestionState"
        }
      },
      "required": [
        "id",
        "sessionId",
        "question",
        "state",
        "choices",
        "createdAt"
      ],
      "type": "object"
    },
    "QuestionState": {
      "enum": [
        "pending",
        "answered",
        "discuss_later"
      ],
      "type": "string"
    },
    "QuestionsReply": {
      "additionalProperties": false,
      "properties": {
        "pendingCount": {
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "questions": {
          "items": {
            "$ref": "#/$defs/QuestionInfo"
          },
          "type": "array"
        }
      },
      "required": [
        "pendingCount",
        "questions"
      ],
      "type": "object"
    },
    "RemoveProject": {
      "additionalProperties": false,
      "properties": {
        "projectId": {
          "type": "string"
        }
      },
      "required": [
        "projectId"
      ],
      "type": "object"
    },
    "RenameProject": {
      "additionalProperties": false,
      "description": "Change only the project's display name, preserving its folder and chats.",
      "properties": {
        "name": {
          "maxLength": 200,
          "minLength": 1,
          "type": "string"
        },
        "projectId": {
          "type": "string"
        }
      },
      "required": [
        "projectId",
        "name"
      ],
      "type": "object"
    },
    "RenameSession": {
      "additionalProperties": false,
      "properties": {
        "name": {
          "maxLength": 200,
          "minLength": 1,
          "type": "string"
        },
        "sessionId": {
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "name"
      ],
      "type": "object"
    },
    "ReorderChats": {
      "additionalProperties": false,
      "description": "One project's chats in the order the owner dragged them to. Chats left out keep their places.",
      "properties": {
        "projectId": {
          "type": "string"
        },
        "sessionIds": {
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "projectId",
        "sessionIds"
      ],
      "type": "object"
    },
    "ReorderProjects": {
      "additionalProperties": false,
      "description": "The projects in the order the owner dragged them to. Projects left out keep their places.",
      "properties": {
        "projectIds": {
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "projectIds"
      ],
      "type": "object"
    },
    "RetryMessage": {
      "additionalProperties": false,
      "properties": {
        "messageId": {
          "pattern": "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
          "type": "string"
        },
        "sessionId": {
          "maxLength": 128,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "messageId"
      ],
      "type": "object"
    },
    "Role": {
      "enum": [
        "user",
        "assistant",
        "system"
      ],
      "type": "string"
    },
    "SecretReply": {
      "additionalProperties": false,
      "description": "What became of a submitted answer. `status` is `saved`, `failed`, `canceled`, `expired` or\n`closed` (already answered), and `message` says it in words for the owner.",
      "properties": {
        "message": {
          "type": "string"
        },
        "status": {
          "type": "string"
        }
      },
      "required": [
        "status",
        "message"
      ],
      "type": "object"
    },
    "SecretRequestedEvent": {
      "additionalProperties": false,
      "properties": {
        "expires_at": {
          "format": "int64",
          "type": "integer"
        },
        "file": {
          "description": "The SOPS file the value would be saved into.",
          "type": "string"
        },
        "key": {
          "description": "The path of keys inside that file.",
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "request_id": {
          "type": "string"
        }
      },
      "required": [
        "request_id",
        "file",
        "key",
        "expires_at"
      ],
      "type": "object"
    },
    "SecretResolvedEvent": {
      "additionalProperties": false,
      "properties": {
        "request_id": {
          "type": "string"
        },
        "status": {
          "description": "`saved`, `canceled`, `expired` or `failed`.",
          "type": "string"
        }
      },
      "required": [
        "request_id",
        "status"
      ],
      "type": "object"
    },
    "SendMessage": {
      "additionalProperties": false,
      "properties": {
        "attachments": {
          "description": "Upload IDs in the order the owner attached them.",
          "items": {
            "type": "string"
          },
          "maxItems": 10,
          "type": "array"
        },
        "content": {
          "description": "May be empty when at least one attachment is present.",
          "maxLength": 65536,
          "type": "string"
        },
        "messageId": {
          "pattern": "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
          "type": "string"
        },
        "sessionId": {
          "maxLength": 128,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "messageId",
        "content"
      ],
      "type": "object"
    },
    "SendMessageResponse": {
      "additionalProperties": false,
      "properties": {
        "accepted": {
          "type": "boolean"
        },
        "messageId": {
          "type": "string"
        },
        "status": {
          "$ref": "#/$defs/SessionStatus"
        }
      },
      "required": [
        "messageId",
        "accepted",
        "status"
      ],
      "type": "object"
    },
    "Sequence": {
      "maximum": 9007199254740991,
      "minimum": 0,
      "type": "integer"
    },
    "SessionCreatedEvent": {
      "additionalProperties": false,
      "properties": {
        "id": {
          "type": "string"
        },
        "parent_session_id": {
          "type": [
            "string",
            "null"
          ]
        },
        "role": {
          "type": [
            "string",
            "null"
          ]
        },
        "root_session_id": {
          "type": [
            "string",
            "null"
          ]
        },
        "working_directory": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "working_directory"
      ],
      "type": "object"
    },
    "SessionInfo": {
      "additionalProperties": false,
      "properties": {
        "agent": {
          "$ref": "#/$defs/Agent"
        },
        "agentSessionId": {
          "type": [
            "string",
            "null"
          ]
        },
        "createdAt": {
          "format": "int64",
          "type": "integer"
        },
        "name": {
          "type": "string"
        },
        "parentSessionId": {
          "type": [
            "string",
            "null"
          ]
        },
        "projectId": {
          "type": "string"
        },
        "role": {
          "type": [
            "string",
            "null"
          ]
        },
        "rootSessionId": {
          "type": [
            "string",
            "null"
          ]
        },
        "sessionId": {
          "type": "string"
        },
        "status": {
          "$ref": "#/$defs/SessionStatus"
        },
        "subagents": {
          "anyOf": [
            {
              "$ref": "#/$defs/SubagentSummary"
            },
            {
              "type": "null"
            }
          ],
          "description": "Present on a root chat; absent on a subagent."
        },
        "workingDirectory": {
          "type": "string"
        }
      },
      "required": [
        "name",
        "sessionId",
        "projectId",
        "agent",
        "workingDirectory",
        "createdAt",
        "status"
      ],
      "type": "object"
    },
    "SessionListEvent": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "deleted",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/Empty"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "created",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SessionCreatedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "renamed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SessionRenamedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "archived",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/ArchiveChangedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "state_summary",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SessionStatus"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "description": "A root chat's subagent summary changed in a way no other list event shows.",
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "subagents_changed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/Empty"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        }
      ]
    },
    "SessionRef": {
      "additionalProperties": false,
      "properties": {
        "sessionId": {
          "maxLength": 128,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        }
      },
      "required": [
        "sessionId"
      ],
      "type": "object"
    },
    "SessionRenamedEvent": {
      "additionalProperties": false,
      "properties": {
        "name": {
          "type": "string"
        }
      },
      "required": [
        "name"
      ],
      "type": "object"
    },
    "SessionStatus": {
      "additionalProperties": false,
      "properties": {
        "accountKey": {
          "description": "Stored provider account key; absent Claude keys mean plan A.",
          "type": [
            "string",
            "null"
          ]
        },
        "accountUsage": {
          "anyOf": [
            {
              "$ref": "#/$defs/AccountUsage"
            },
            {
              "type": "null"
            }
          ]
        },
        "archived": {
          "type": "boolean"
        },
        "dispatching": {
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "effort": {
          "type": [
            "string",
            "null"
          ]
        },
        "heldReason": {
          "description": "Why the queue is held, when the last driver error caused it; shown beside Resume.",
          "type": [
            "string",
            "null"
          ]
        },
        "jobs": {
          "anyOf": [
            {
              "$ref": "#/$defs/JobSnapshot"
            },
            {
              "type": "null"
            }
          ]
        },
        "model": {
          "type": "string"
        },
        "promptSuggestion": {
          "description": "Latest provider prediction; transient and absent when unavailable.",
          "type": [
            "string",
            "null"
          ]
        },
        "queueHeld": {
          "type": "boolean"
        },
        "queued": {
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "rejected": {
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "running": {
          "type": "boolean"
        },
        "selectionPending": {
          "type": "boolean"
        },
        "stopping": {
          "type": "boolean"
        },
        "turnActive": {
          "type": "boolean"
        },
        "uncertain": {
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "usage": {
          "anyOf": [
            {
              "$ref": "#/$defs/UsageSnapshot"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "selectionPending",
        "queueHeld",
        "running",
        "turnActive",
        "stopping",
        "queued",
        "dispatching",
        "uncertain",
        "rejected",
        "archived",
        "model"
      ],
      "type": "object"
    },
    "SetModel": {
      "additionalProperties": false,
      "properties": {
        "effort": {
          "type": [
            "string",
            "null"
          ]
        },
        "model": {
          "type": "string"
        },
        "sessionId": {
          "maxLength": 128,
          "minLength": 1,
          "pattern": "\\S",
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "model"
      ],
      "type": "object"
    },
    "StoredEvent": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "session_renamed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SessionRenamedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "session_created",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SessionCreatedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "message_appended",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/MessageAppendedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "delivery_changed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/DeliveryChangedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "agent_session_changed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/AgentSessionChangedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "archive_changed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/ArchiveChangedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "model_effort_changed",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/ModelEffortChangedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "delivery_rejected",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/DeliveryRejectedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "agent_event",
              "type": "string"
            },
            "payload": true,
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "driver_error",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/DriverErrorEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "model_error",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/DriverErrorEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "state_summary",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SessionStatus"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "session_stopping",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/Empty"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "session_stopped",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/Empty"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "delivery_timeout",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/Empty"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "stop_timeout",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/Empty"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "questions_changed",
              "type": "string"
            },
            "payload": true,
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "secret_requested",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SecretRequestedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "createdAt": {
              "format": "int64",
              "type": "integer"
            },
            "kind": {
              "const": "secret_resolved",
              "type": "string"
            },
            "payload": {
              "$ref": "#/$defs/SecretResolvedEvent"
            },
            "seq": {
              "$ref": "#/$defs/Sequence"
            },
            "sessionId": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "sessionId",
            "seq",
            "createdAt",
            "payload"
          ],
          "type": "object"
        }
      ]
    },
    "SubagentHistory": {
      "additionalProperties": false,
      "description": "Read a subagent transcript belonging to this chat. Cursors are transcript byte positions.",
      "properties": {
        "agentKey": {
          "type": "string"
        },
        "before": {
          "anyOf": [
            {
              "$ref": "#/$defs/Sequence"
            },
            {
              "type": "null"
            }
          ]
        },
        "sessionId": {
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "agentKey"
      ],
      "type": "object"
    },
    "SubagentHistoryPage": {
      "additionalProperties": false,
      "properties": {
        "agentKey": {
          "type": "string"
        },
        "cursor": {
          "$ref": "#/$defs/Sequence"
        },
        "events": {
          "items": {
            "$ref": "#/$defs/StoredEvent"
          },
          "type": "array"
        },
        "hasMore": {
          "type": "boolean"
        },
        "sessionId": {
          "type": "string"
        }
      },
      "required": [
        "sessionId",
        "agentKey",
        "events",
        "hasMore",
        "cursor"
      ],
      "type": "object"
    },
    "SubagentSummary": {
      "additionalProperties": false,
      "description": "Counts a root's subagents for its sidebar entry and alert badge.",
      "properties": {
        "failures": {
          "description": "Subagents whose latest turn ended in failure.",
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "open": {
          "description": "Subagents that are not archived.",
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "questions": {
          "description": "Unanswered questions from subagents.",
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        },
        "reportFailures": {
          "description": "Subagents with a failed report delivery recorded in their history.",
          "format": "uint32",
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "open",
        "questions",
        "failures",
        "reportFailures"
      ],
      "type": "object"
    },
    "SubmitSecret": {
      "additionalProperties": false,
      "description": "The owner's answer to a chat's request for a credential: the value to save, or a refusal.\nDeliberately not `Debug`, so the value cannot reach a log by accident.",
      "properties": {
        "cancel": {
          "description": "Decline the request (use instead of `value`).",
          "type": [
            "boolean",
            "null"
          ]
        },
        "requestId": {
          "type": "string"
        },
        "value": {
          "description": "The credential to save. Omit it to decline instead.",
          "maxLength": 65536,
          "minLength": 1,
          "type": [
            "string",
            "null"
          ]
        }
      },
      "required": [
        "requestId"
      ],
      "type": "object"
    },
    "SubscribePush": {
      "additionalProperties": false,
      "properties": {
        "auth": {
          "type": "string"
        },
        "endpoint": {
          "type": "string"
        },
        "p256dh": {
          "type": "string"
        }
      },
      "required": [
        "endpoint",
        "p256dh",
        "auth"
      ],
      "type": "object"
    },
    "UnsubscribePush": {
      "additionalProperties": false,
      "properties": {
        "endpoint": {
          "type": "string"
        }
      },
      "required": [
        "endpoint"
      ],
      "type": "object"
    },
    "UsageSnapshot": {
      "properties": {
        "context": {
          "anyOf": [
            {
              "$ref": "#/$defs/ContextUsage"
            },
            {
              "type": "null"
            }
          ]
        },
        "contextUpdatedAt": {
          "format": "int64",
          "type": [
            "integer",
            "null"
          ]
        }
      },
      "type": "object"
    }
  },
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "add_project": {
      "$ref": "#/$defs/AddProject"
    },
    "answer_question": {
      "$ref": "#/$defs/AnswerQuestion"
    },
    "attachment": {
      "$ref": "#/$defs/AttachmentInfo"
    },
    "chat_history": {
      "$ref": "#/$defs/ChatHistory"
    },
    "chat_history_page": {
      "$ref": "#/$defs/ChatHistoryPage"
    },
    "chat_presence": {
      "$ref": "#/$defs/ChatPresence"
    },
    "chat_presence_reply": {
      "$ref": "#/$defs/ChatPresenceReply"
    },
    "command": {
      "$ref": "#/$defs/CommandResponse"
    },
    "create_session": {
      "$ref": "#/$defs/CreateSession"
    },
    "delivery_state": {
      "$ref": "#/$defs/DeliveryState"
    },
    "directory_listing": {
      "$ref": "#/$defs/DirectoryListing"
    },
    "discuss_question_later": {
      "$ref": "#/$defs/DiscussQuestionLater"
    },
    "empty": {
      "$ref": "#/$defs/Empty"
    },
    "error": {
      "$ref": "#/$defs/ErrorInfo"
    },
    "event_page": {
      "$ref": "#/$defs/EventPage"
    },
    "list_directories": {
      "$ref": "#/$defs/ListDirectories"
    },
    "list_sessions": {
      "$ref": "#/$defs/ListSessionsResponse"
    },
    "project": {
      "$ref": "#/$defs/ProjectInfo"
    },
    "push_config": {
      "$ref": "#/$defs/PushConfigReply"
    },
    "questions": {
      "$ref": "#/$defs/QuestionsReply"
    },
    "remove_project": {
      "$ref": "#/$defs/RemoveProject"
    },
    "rename_project": {
      "$ref": "#/$defs/RenameProject"
    },
    "rename_session": {
      "$ref": "#/$defs/RenameSession"
    },
    "reorder_chats": {
      "$ref": "#/$defs/ReorderChats"
    },
    "reorder_projects": {
      "$ref": "#/$defs/ReorderProjects"
    },
    "retry_message": {
      "$ref": "#/$defs/RetryMessage"
    },
    "secret_reply": {
      "$ref": "#/$defs/SecretReply"
    },
    "send_message": {
      "$ref": "#/$defs/SendMessage"
    },
    "send_response": {
      "$ref": "#/$defs/SendMessageResponse"
    },
    "session_info": {
      "$ref": "#/$defs/SessionInfo"
    },
    "session_list_event": {
      "$ref": "#/$defs/SessionListEvent"
    },
    "session_ref": {
      "$ref": "#/$defs/SessionRef"
    },
    "set_model": {
      "$ref": "#/$defs/SetModel"
    },
    "subagent_history": {
      "$ref": "#/$defs/SubagentHistory"
    },
    "subagent_history_page": {
      "$ref": "#/$defs/SubagentHistoryPage"
    },
    "submit_secret": {
      "$ref": "#/$defs/SubmitSecret"
    },
    "subscribe_push": {
      "$ref": "#/$defs/SubscribePush"
    },
    "unsubscribe_push": {
      "$ref": "#/$defs/UnsubscribePush"
    }
  },
  "required": [
    "session_list_event",
    "empty",
    "session_ref",
    "subagent_history",
    "subagent_history_page",
    "create_session",
    "rename_session",
    "send_message",
    "attachment",
    "retry_message",
    "set_model",
    "add_project",
    "rename_project",
    "list_directories",
    "directory_listing",
    "remove_project",
    "reorder_projects",
    "reorder_chats",
    "push_config",
    "subscribe_push",
    "unsubscribe_push",
    "chat_presence",
    "chat_presence_reply",
    "questions",
    "answer_question",
    "discuss_question_later",
    "submit_secret",
    "secret_reply",
    "project",
    "error",
    "list_sessions",
    "session_info",
    "send_response",
    "command",
    "event_page",
    "chat_history",
    "chat_history_page",
    "delivery_state"
  ],
  "title": "ContractSchemas",
  "type": "object",
  "x-maxBodyBytes": 1048576
} as const;
export function parseEmpty(value: unknown): Empty { return decode(contractSchema, "Empty", value) as Empty; }
export function parseSessionRef(value: unknown): SessionRef { return decode(contractSchema, "SessionRef", value) as SessionRef; }
export function parseCreateSession(value: unknown): CreateSession { return decode(contractSchema, "CreateSession", value) as CreateSession; }
export function parseRenameSession(value: unknown): RenameSession { return decode(contractSchema, "RenameSession", value) as RenameSession; }
export function parseSendMessage(value: unknown): SendMessage { return decode(contractSchema, "SendMessage", value) as SendMessage; }
export function parseAttachmentInfo(value: unknown): AttachmentInfo { return decode(contractSchema, "AttachmentInfo", value) as AttachmentInfo; }
export function parseRetryMessage(value: unknown): RetryMessage { return decode(contractSchema, "RetryMessage", value) as RetryMessage; }
export function parseSetModel(value: unknown): SetModel { return decode(contractSchema, "SetModel", value) as SetModel; }
export function parseAddProject(value: unknown): AddProject { return decode(contractSchema, "AddProject", value) as AddProject; }
export function parseRenameProject(value: unknown): RenameProject { return decode(contractSchema, "RenameProject", value) as RenameProject; }
export function parseListDirectories(value: unknown): ListDirectories { return decode(contractSchema, "ListDirectories", value) as ListDirectories; }
export function parseDirectoryListing(value: unknown): DirectoryListing { return decode(contractSchema, "DirectoryListing", value) as DirectoryListing; }
export function parseRemoveProject(value: unknown): RemoveProject { return decode(contractSchema, "RemoveProject", value) as RemoveProject; }
export function parseReorderProjects(value: unknown): ReorderProjects { return decode(contractSchema, "ReorderProjects", value) as ReorderProjects; }
export function parseReorderChats(value: unknown): ReorderChats { return decode(contractSchema, "ReorderChats", value) as ReorderChats; }
export function parsePushConfigReply(value: unknown): PushConfigReply { return decode(contractSchema, "PushConfigReply", value) as PushConfigReply; }
export function parseSubscribePush(value: unknown): SubscribePush { return decode(contractSchema, "SubscribePush", value) as SubscribePush; }
export function parseUnsubscribePush(value: unknown): UnsubscribePush { return decode(contractSchema, "UnsubscribePush", value) as UnsubscribePush; }
export function parseChatPresence(value: unknown): ChatPresence { return decode(contractSchema, "ChatPresence", value) as ChatPresence; }
export function parseChatPresenceReply(value: unknown): ChatPresenceReply { return decode(contractSchema, "ChatPresenceReply", value) as ChatPresenceReply; }
export function parseQuestionInfo(value: unknown): QuestionInfo { return decode(contractSchema, "QuestionInfo", value) as QuestionInfo; }
export function parseQuestionState(value: unknown): QuestionState { return decode(contractSchema, "QuestionState", value) as QuestionState; }
export function parseDiscussQuestionLater(value: unknown): DiscussQuestionLater { return decode(contractSchema, "DiscussQuestionLater", value) as DiscussQuestionLater; }
export function parseQuestionsReply(value: unknown): QuestionsReply { return decode(contractSchema, "QuestionsReply", value) as QuestionsReply; }
export function parseAnswerQuestion(value: unknown): AnswerQuestion { return decode(contractSchema, "AnswerQuestion", value) as AnswerQuestion; }
export function parseSubmitSecret(value: unknown): SubmitSecret { return decode(contractSchema, "SubmitSecret", value) as SubmitSecret; }
export function parseSecretReply(value: unknown): SecretReply { return decode(contractSchema, "SecretReply", value) as SecretReply; }
export function parseProjectInfo(value: unknown): ProjectInfo { return decode(contractSchema, "ProjectInfo", value) as ProjectInfo; }
export function parseErrorInfo(value: unknown): ErrorInfo { return decode(contractSchema, "ErrorInfo", value) as ErrorInfo; }
export function parseListSessionsResponse(value: unknown): ListSessionsResponse { return decode(contractSchema, "ListSessionsResponse", value) as ListSessionsResponse; }
export function parseSessionInfo(value: unknown): SessionInfo { return decode(contractSchema, "SessionInfo", value) as SessionInfo; }
export function parseSessionStatus(value: unknown): SessionStatus { return decode(contractSchema, "SessionStatus", value) as SessionStatus; }
export function parseSendMessageResponse(value: unknown): SendMessageResponse { return decode(contractSchema, "SendMessageResponse", value) as SendMessageResponse; }
export function parseCommandResponse(value: unknown): CommandResponse { return decode(contractSchema, "CommandResponse", value) as CommandResponse; }
export function parseChatHistory(value: unknown): ChatHistory { return decode(contractSchema, "ChatHistory", value) as ChatHistory; }
export function parseChatHistoryPage(value: unknown): ChatHistoryPage { return decode(contractSchema, "ChatHistoryPage", value) as ChatHistoryPage; }
export function parseSubagentHistory(value: unknown): SubagentHistory { return decode(contractSchema, "SubagentHistory", value) as SubagentHistory; }
export function parseSubagentHistoryPage(value: unknown): SubagentHistoryPage { return decode(contractSchema, "SubagentHistoryPage", value) as SubagentHistoryPage; }
export function parseEventPage(value: unknown): EventPage { return decode(contractSchema, "EventPage", value) as EventPage; }
export function parseDeliveryState(value: unknown): DeliveryState { return decode(contractSchema, "DeliveryState", value) as DeliveryState; }
export function parseStoredEvent(value: unknown): StoredEvent { return decode(contractSchema, "StoredEvent", value) as StoredEvent; }
export function parseSequence(value: unknown): Sequence { return decode(contractSchema, "Sequence", value) as Sequence; }
export function parseSessionListEvent(value: unknown): SessionListEvent { return decode(contractSchema, "SessionListEvent", value) as SessionListEvent; }
export const limits = { maxChatNameChars: 200, maxAttachments: 10, maxBodyBytes: 1048576, maxUploadBytes: 52428800, maxMessageChars: 65536, defaultModel: "opus", defaultEffort: "high", historyPageSize: 50, historyMaxRows: 400, historyResumeLatestMs: 900000, historyScrollIntentMs: 1000, historyScrollThreshold: 160, longPressMs: 500, longPressMovePx: 8, maxProjectNameChars: 200, notificationHeartbeatMs: 15000, browserConnectTimeoutMs: 15000, subagentPollMs: 2000, jobsVisibleLimit: 50, queuedMessageMaxAgeHours: 6, usageHighPercent: 90 } as const;
