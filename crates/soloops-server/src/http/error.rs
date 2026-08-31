use super::*;

#[derive(Debug)]
pub(super) struct AppError {
    status: StatusCode,
    body: ApiErrorResponse,
}

impl AppError {
    pub(super) fn new(status: StatusCode, code: &str, message: &str) -> Self {
        Self {
            status,
            body: ApiErrorResponse::new(code, message),
        }
    }

    pub(super) fn unauthorized(code: &str, message: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code, message)
    }

    pub(super) fn forbidden(code: &str, message: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    pub(super) fn not_found(code: &str, message: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub(super) fn conflict(code: &str, message: &str) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        error!(%error, "server operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Unexpected server error",
        )
    }

    pub(super) fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts; try again later",
        )
    }

    pub(super) fn invalid_payload(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ApiErrorResponse::new("invalid_request", "Request parsing failed")
                .with_details(json!({ "reason": message })),
        }
    }

    pub(super) fn validation(field: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ApiErrorResponse::new("invalid_request", "Request validation failed")
                .with_details(json!([{ "field": field, "message": message }])),
        }
    }
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidEventPayload { sequence, .. } => {
                error!(sequence, "persisted event payload is corrupt");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: ApiErrorResponse::new(
                        "event_stream_corrupt",
                        "The persisted event stream is corrupt and requires operator repair",
                    )
                    .with_details(json!({ "sequence": sequence, "retryable": false })),
                }
            }
            error => {
                error!(%error, "storage request failed");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Unexpected server error",
                )
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
