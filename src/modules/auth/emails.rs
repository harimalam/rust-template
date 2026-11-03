//! Email templates for the auth flows.

use crate::infra::mail::Email;

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `base_url` should point at the frontend route that reads `?token=` and POSTs it to
/// `/api/v1/auth/verify-email`. The raw token is also in the body for API-only clients.
pub fn verification(to: &str, name: &str, base_url: &str, token: &str) -> Email {
    let link = format!("{base_url}/verify-email?token={token}");
    let name_html = escape_html(name);
    Email {
        to: to.to_owned(),
        subject: "Verify your email address".into(),
        text: format!(
            "Hi {name},\n\nConfirm your email address by opening:\n{link}\n\n\
             Or submit this token to POST /api/v1/auth/verify-email:\n{token}\n\n\
             If you didn't create an account, ignore this email."
        ),
        html: format!(
            "<p>Hi {name_html},</p><p><a href=\"{link}\">Confirm your email address</a></p>\
             <p>Or submit this token to <code>POST /api/v1/auth/verify-email</code>:<br>\
             <code>{token}</code></p><p>If you didn't create an account, ignore this email.</p>"
        ),
    }
}

pub fn password_reset(to: &str, name: &str, base_url: &str, token: &str) -> Email {
    let link = format!("{base_url}/reset-password?token={token}");
    let name_html = escape_html(name);
    Email {
        to: to.to_owned(),
        subject: "Reset your password".into(),
        text: format!(
            "Hi {name},\n\nReset your password by opening:\n{link}\n\n\
             Or submit this token to POST /api/v1/auth/reset-password:\n{token}\n\n\
             If you didn't request this, ignore this email; your password is unchanged."
        ),
        html: format!(
            "<p>Hi {name_html},</p><p><a href=\"{link}\">Reset your password</a></p>\
             <p>Or submit this token to <code>POST /api/v1/auth/reset-password</code>:<br>\
             <code>{token}</code></p>\
             <p>If you didn't request this, ignore this email; your password is unchanged.</p>"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_body_escapes_user_supplied_name() {
        let email = verification(
            "a@example.com",
            "<script>alert(1)</script>",
            "https://app",
            "tok",
        );
        assert!(!email.html.contains("<script>"));
        assert!(email.html.contains("&lt;script&gt;"));
        assert!(email.text.contains("https://app/verify-email?token=tok"));
    }
}
