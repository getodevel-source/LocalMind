use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Escapar texto para incrustarlo en XML del toast (P20). Sin pánicos.
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Armar el comando PowerShell del toast sin lanzarlo (puerta testeable).
/// Devuelve `None` si los avisos están apagados para ese evento.
pub fn toast_command(
    enabled: bool,
    flag_on: bool,
    title: &str,
    body: &str,
    tag: &str,
) -> Option<String> {
    if !enabled || !flag_on {
        return None;
    }
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape_xml(title),
        escape_xml(body),
    );
    let ps_xml = xml.replace('\'', "''");
    let ps_tag = tag.replace('\'', "''");
    Some(format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null; $xml = New-Object Windows.Data.Xml.Dom.XmlDocument; $xml.LoadXml('{}'); $toast = New-Object Windows.UI.Notifications.ToastNotification($xml); $toast.Tag = '{}'; [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('LocalMind').Show($toast)",
        ps_xml, ps_tag
    ))
}

/// Avisar por toast de Windows (P20): PowerShell desacoplado, sin ventana de
/// consola, sin espera. Si el spawn falla, se registra y se sigue: los avisos
/// nunca deben romper la app. `log_on_fail` recibe el motivo (normalmente un
/// `mgr.log(...)` del llamador).
pub fn notify(title: &str, body: &str, tag: &str, log_on_fail: impl FnOnce(&str)) {
    let script = match toast_command(true, true, title, body, tag) {
        Some(s) => s,
        None => return,
    };
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &script,
    ]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    // Desacoplado: no se espera el hijo (el toast vive en el SO).
    match cmd.spawn() {
        Ok(_) => {}
        Err(e) => log_on_fail(&format!(
            "[LocalMind] Aviso de escritorio no enviado ({}).",
            e
        )),
    }
}

/// Puerta de avisos con la config viva (P20): `enabled` general + flag del evento.
pub fn should_notify(enabled: bool, flag_on: bool) -> bool {
    enabled && flag_on
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_disabled_or_flag_off_builds_nothing() {
        assert!(toast_command(false, true, "t", "b", "tag").is_none());
        assert!(toast_command(true, false, "t", "b", "tag").is_none());
        assert!(toast_command(false, false, "t", "b", "tag").is_none());
        assert!(toast_command(true, true, "t", "b", "tag").is_some());
        assert!(!should_notify(false, true));
        assert!(!should_notify(true, false));
        assert!(should_notify(true, true));
    }

    #[test]
    fn xml_escapes_body_with_markup_and_quotes() {
        let cmd = toast_command(true, true, "Motor listo", "a<b>&\"c\"'d'", "ready").expect("cmd");
        assert!(cmd.contains("a&lt;b&gt;&amp;&quot;c&quot;&apos;d&apos;"));
        assert!(!cmd.contains("a<b>"));
    }
}
