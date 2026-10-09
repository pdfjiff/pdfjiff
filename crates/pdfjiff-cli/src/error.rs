use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub hint: &'static str,
    #[serde(skip)]
    pub exit: u8,
}
impl Failure {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        hint: &'static str,
        exit: u8,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            hint,
            exit,
        }
    }
    pub fn processing(message: impl Into<String>) -> Self {
        Self::new(
            "PROCESSING_FAILED",
            message,
            "Inspect the input and try a supported, unprotected PDF.",
            7,
        )
    }
}
impl From<pdfjiff_core::CoreError> for Failure {
    fn from(error: pdfjiff_core::CoreError) -> Self {
        use pdfjiff_core::CoreError;
        match error {
            CoreError::InvalidInput(message) => Self::new(
                "INVALID_PDF",
                message,
                "Use a valid PDF with readable pages.",
                3,
            ),
            CoreError::PasswordRequired => Self::new(
                "PASSWORD_REQUIRED",
                error.to_string(),
                "Decrypt a copy using your password before retrying.",
                4,
            ),
            CoreError::Processing(message) => Self::processing(message),
        }
    }
}
pub type Result<T> = std::result::Result<T, Failure>;
