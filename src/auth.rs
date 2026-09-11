use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

const NM_API_BASE: &str = "https://apis.netmarble.com/cpplauncher/api/external";
#[allow(dead_code)]
pub const NM_MEMBERS_AUTH_URL: &str = "https://members.netmarble.com/auth";
pub const NM_CLIENT_ID: &str = "mq5RG0PGw6ipw35A";
const NM_LAUNCHER_CH: &str = "ypWjRL2aNi";
const NM_LAUNCHER_VER: &str = "1.7.0";
const GAME_CODE: &str = "monster2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub launcher_token: String,
    pub channel: String,
    pub channel_code: u32,
    pub player_id: String,
    pub profile_name: String,
    pub profile_img_url: String,
    pub expires_at: i64,
    pub device_key: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatusPayload {
    pub is_logged_in: bool,
    pub channel: String,
    pub player_id: String,
    pub profile_name: String,
    pub profile_img_url: String,
    pub expires_at: i64,
}

impl Default for AuthStatusPayload {
    fn default() -> Self {
        Self {
            is_logged_in: false,
            channel: String::new(),
            player_id: String::new(),
            profile_name: String::new(),
            profile_img_url: String::new(),
            expires_at: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetmarbleUserData {
    #[serde(default, rename = "netmarbleId")]
    pub netmarble_id: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default, rename = "mailAddress")]
    pub mail_address: Option<String>,
    #[serde(default, rename = "joinedCountryCode")]
    pub joined_country_code: Option<String>,
    #[serde(default, rename = "accessedChannelCode")]
    pub accessed_channel_code: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPayload {
    pub channel: String,
    #[serde(rename = "idToken", default)]
    pub id_token: Option<String>,
    #[serde(rename = "googleClientId", default)]
    pub google_client_id: Option<String>,
    #[serde(rename = "accessToken", default)]
    pub access_token: Option<String>,
    #[serde(rename = "user", default)]
    pub user: Option<String>,
    #[serde(rename = "code", default)]
    pub code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NetmarbleApiResponse<T> {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: Option<T>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GameProfileData {
    #[allow(dead_code)]
    #[serde(default, rename = "playerId")]
    pub player_id: Option<String>,
    #[serde(default, rename = "profileName")]
    pub profile_name: Option<String>,
    #[serde(default, rename = "profileImgURL")]
    pub profile_img_url: Option<String>,
}

pub struct AuthManager;

impl AuthManager {
    pub fn auth_file_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/home/shidox"))
                    .join(".config")
            })
            .join("stardive-launcher");
        let _ = fs::create_dir_all(&config_dir);
        config_dir.join("auth.json")
    }

    /// Lee el `NMDeviceKey` del prefijo de Wine.
    ///
    /// IMPORTANTE: El `nmDeviceKey` que el juego envía a la API de Netmarble NO es
    /// lo mismo que el `deviceKey` (UUID del launcher). Es una clave generada por el
    /// SDK del juego (`NMSDKService`) y guardada en la sección `Netmarble dev\monster2`
    /// del registro, bajo el nombre `DeviceKey` (32 hex, formato MD5).
    ///
    /// El servidor de Netmarble valida esta combinación al emitir/refrescar tokens:
    /// `{"deviceKey": <uuid-del-launcher>, "nmDeviceKey": <md5-del-sdk>}` → HTTP 200.
    ///
    /// Descubierto empíricamente contra la API real de Netmarble (2026-09-11).
    #[allow(dead_code)]
    pub fn read_nm_device_key_from_wine_registry() -> Option<String> {
        Self::for_each_user_reg(Self::parse_nm_device_key_from_reg_content)
    }

    /// Lee el `NMDeviceKey` de la sección `Netmarble\NetmarbleSDK` (el UUID del
    /// launcher adoptado en instalaciones nuevas, o el valor del SDK en instalaciones
    /// antiguas). Este es el valor que va en el campo `deviceKey` del body.
    fn read_launcher_device_key_from_wine_registry() -> Option<String> {
        Self::for_each_user_reg(Self::parse_launcher_device_key_from_reg_content)
    }

    /// Extrae el `nmDeviceKey` (MD5 del SDK del juego) del contenido de un `user.reg`.
    ///
    /// Busca en la sección `[Software\\Netmarble dev\\monster2]` la clave `DeviceKey`.
    /// Esta clave la escribe el propio SDK (`NMSDKService`) del juego la primera vez
    /// que corre, y es la que el servidor de Netmarble valida como `nmDeviceKey`.
    /// Función pura → testeable sin disco.
    fn parse_nm_device_key_from_reg_content(content: &str) -> Option<String> {
        Self::find_value_in_reg_section(
            content,
            &[
                "[Software\\\\Netmarble dev\\\\monster2]",
                "[Software\\Netmarble dev\\monster2]",
            ],
            &["DeviceKey"],
        )
    }

    /// Extrae el `deviceKey` (UUID del launcher) del contenido de un `user.reg`.
    ///
    /// Busca en la sección `[Software\\Netmarble\\NetmarbleSDK]` la clave `NMDeviceKey`.
    /// Función pura → testeable sin disco.
    fn parse_launcher_device_key_from_reg_content(content: &str) -> Option<String> {
        Self::find_value_in_reg_section(
            content,
            &[
                "[Software\\\\Netmarble\\\\NetmarbleSDK]",
                "[Software\\Netmarble\\NetmarbleSDK]",
            ],
            &["NMDeviceKey", "DeviceKey"],
        )
    }

    /// Busca, dentro de la sección `sections` (probando variantes de escape), la
    /// primera clave de `keys` que tenga valor no vacío. Devuelve ese valor.
    ///
    /// No "sangra" a la sección siguiente: recorta el bloque hasta el próximo `\n[`.
    fn find_value_in_reg_section(content: &str, sections: &[&str], keys: &[&str]) -> Option<String> {
        for section in sections {
            let Some(sec_idx) = content.find(section) else {
                continue;
            };
            let rest = &content[sec_idx..];
            let block_end = rest[section.len()..]
                .find("\n[")
                .map(|o| section.len() + o)
                .unwrap_or(rest.len());
            let block = &rest[..block_end];

            for key in keys {
                let needle = format!("\"{}\"=\"", key);
                let Some(ki) = block.find(&needle) else {
                    continue;
                };
                let start = ki + needle.len();
                if let Some(end) = block[start..].find('"') {
                    let val = block[start..start + end].trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
            }
        }
        None
    }

    /// Iterador de candidatos al `user.reg` del prefijo (config + fallbacks).
    /// Aplica `f` a cada contenido legible y retorna el primer `Some`.
    fn for_each_user_reg<F: FnMut(&str) -> Option<String>>(mut f: F) -> Option<String> {
        let cfg_path = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/home/shidox"))
                    .join(".config")
            })
            .join("stardive-launcher")
            .join("config.json");

        let prefix = fs::read_to_string(&cfg_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("prefix_dir").and_then(|p| p.as_str()).map(PathBuf::from));

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = prefix {
            candidates.push(p.join("pfx").join("user.reg"));
            candidates.push(p.join("user.reg"));
        }
        if let Some(home) = dirs::home_dir() {
            let def = home.join("Games").join("mongil-star-dive");
            candidates.push(def.join("pfx").join("user.reg"));
            candidates.push(def.join("user.reg"));
        }

        for reg_path in candidates {
            let Ok(content) = fs::read_to_string(&reg_path) else {
                continue;
            };
            if let Some(v) = f(&content) {
                return Some(v);
            }
        }
        None
    }

    pub fn get_or_create_device_key() -> String {
        let path = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/home/shidox"))
                    .join(".config")
            })
            .join("stardive-launcher")
            .join("device_key");

        // 1) ADOPTAR el UUID del launcher almacenado en `Netmarble\NetmarbleSDK` →
        //    `NMDeviceKey`. Es el que va en el campo `deviceKey` de las llamadas
        //    de canje/refresh de la API de Netmarble. Si no existe en el registro,
        //    caemos al archivo local persistido y, por último, generamos uno nuevo.
        if let Some(reg_key) = Self::read_launcher_device_key_from_wine_registry() {
            let _ = fs::write(&path, &reg_key);
            return reg_key;
        }

        // 2) Fallback: clave persistida localmente (instalación sin registro aún).
        if let Ok(key) = fs::read_to_string(&path) {
            let trimmed = key.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        // 3) Último recurso: generar una nueva (solo si el juego nunca corrió).
        let new_key = Uuid::new_v4().to_string();
        let _ = fs::write(&path, &new_key);
        new_key
    }

    pub fn load_session() -> Result<Option<AuthSession>> {
        let path = Self::auth_file_path();
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read_to_string(&path)?;
        match serde_json::from_str::<AuthSession>(&data) {
            Ok(session) => {
                let now = chrono::Utc::now().timestamp();
                if session.expires_at > 0 && session.expires_at < now {
                    eprintln!("Sesión de Netmarble expirada. Se requiere iniciar sesión nuevamente.");
                    let _ = fs::remove_file(&path);
                    return Ok(None);
                }
                Ok(Some(session))
            }
            Err(e) => {
                eprintln!("Error al leer auth.json: {}", e);
                Ok(None)
            }
        }
    }

    pub fn save_session(session: &AuthSession) -> Result<()> {
        if session.launcher_token.starts_with('/') || session.launcher_token.len() < 20 {
            return Err(anyhow!(
                "Rechazado guardado de sesión con token inválido ('{}')",
                session.launcher_token
            ));
        }
        let path = Self::auth_file_path();
        let json = serde_json::to_string_pretty(session)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn clear_session() -> Result<()> {
        let path = Self::auth_file_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn get_status() -> AuthStatusPayload {
        if let Ok(Some(session)) = Self::load_session() {
            AuthStatusPayload {
                is_logged_in: true,
                channel: session.channel,
                player_id: session.player_id,
                profile_name: session.profile_name,
                profile_img_url: session.profile_img_url,
                expires_at: session.expires_at,
            }
        } else {
            AuthStatusPayload::default()
        }
    }

    /// Canjea los tokens del proveedor (Google, Email, Apple) por el token del launcher (JWT).
    ///
    /// IMPORTANTE: la API de Netmarble exige DOS claves de dispositivo en el body:
    ///   - `deviceKey`: el UUID del launcher (`NMDeviceKey` de `Netmarble\NetmarbleSDK`).
    ///   - `nmDeviceKey`: el MD5 que el SDK del juego escribió en
    ///     `Netmarble dev\monster2` → `DeviceKey`.
    ///
    /// Si `nmDeviceKey` falta o está mal, el servidor responde `1210 invalid device key`
    /// aunque el resto del body sea correcto. Descubierto empíricamente (2026-09-11).
    #[allow(dead_code)]
    pub async fn exchange_channel_token(
        payload: &ChannelPayload,
        device_key: &str,
        nm_device_key: &str,
    ) -> Result<AuthSession> {
        let client = reqwest::Client::builder().build()?;
        let url = format!("{}/cpp-auth/game/{}/token", NM_API_BASE, GAME_CODE);

        let (channel_code, req_body) = match payload.channel.as_str() {
            "google" => {
                let id_token = payload
                    .id_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("idToken faltante para Google"))?;
                let client_id = payload.google_client_id.as_deref().unwrap_or_default();
                (
                    29u32,
                    serde_json::json!({
                        "deviceKey": device_key,
                        "nmDeviceKey": nm_device_key,
                        "channelCode": 29,
                        "googleIdToken": id_token,
                        "googleClientId": client_id,
                        "googlePlayerId": ""
                    }),
                )
            }
            "email" => {
                let access_token = payload
                    .access_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("accessToken faltante para Correo Netmarble"))?;
                (
                    20u32,
                    serde_json::json!({
                        "deviceKey": device_key,
                        "nmDeviceKey": nm_device_key,
                        "channelCode": 20,
                        "channelAccessToken": access_token
                    }),
                )
            }
            "apple" => {
                let id_token = payload
                    .id_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("idToken faltante para Apple"))?;
                (
                    26u32,
                    serde_json::json!({
                        "deviceKey": device_key,
                        "nmDeviceKey": nm_device_key,
                        "channelCode": 26,
                        "user": payload.user.as_deref().unwrap_or_default(),
                        "authorizationCode": payload.code.as_deref().unwrap_or_default(),
                        "identityToken": id_token
                    }),
                )
            }
            other => return Err(anyhow!("Canal de autenticación desconocido: {}", other)),
        };

        println!("Canjeando token con Netmarble Auth (Canal: {})...", payload.channel);

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-NM-LAUNCHER-CH", NM_LAUNCHER_CH)
            .header("X-NM-LAUNCHER-LANG", "es")
            .header("X-NM-LAUNCHER-OS", "windows")
            .header("X-NM-LAUNCHER-OS-VERSION", "10.0.19045")
            .header("X-NM-LAUNCHER-ARCH", "x64")
            .header("X-NM-LAUNCHER-VER", NM_LAUNCHER_VER)
            .header("X-NM-LAUNCHER-SER-TY", "nm")
            .json(&req_body)
            .send()
            .await
            .context("Fallo en la petición HTTP a cpp-auth/token")?;

        let status = resp.status();
        let text = resp.text().await.context("Fallo al leer respuesta HTTP")?;

        if !status.is_success() {
            return Err(anyhow!(
                "Netmarble API devolvió HTTP {}: {}",
                status,
                text
            ));
        }

        let api_resp: NetmarbleApiResponse<String> = serde_json::from_str(&text)
            .with_context(|| format!("Respuesta no válida de Netmarble API: {}", text))?;

        if api_resp.code != 0 {
            return Err(anyhow!(
                "Error de Netmarble Auth (Código {}): {}",
                api_resp.code,
                api_resp.msg.unwrap_or_else(|| "Error desconocido".to_string())
            ));
        }

        let launcher_token = api_resp
            .data
            .ok_or_else(|| anyhow!("No se recibió launcherToken en la respuesta de Netmarble"))?;

        // Decodificar JWT para claims (expiración, player_id / sub)
        let (player_id, expires_at) = Self::decode_jwt_claims(&launcher_token);

        // Consultar el perfil del jugador
        let profile = Self::fetch_game_profile(&client, &launcher_token).await.ok();
        let profile_name = profile
            .as_ref()
            .and_then(|p| p.profile_name.clone())
            .unwrap_or_else(|| {
                if !player_id.is_empty() {
                    format!("Piloto #{}", &player_id[..player_id.len().min(6)])
                } else {
                    "Piloto Estelar".to_string()
                }
            });

        let profile_img_url = profile
            .as_ref()
            .and_then(|p| p.profile_img_url.clone())
            .unwrap_or_default();

        let session = AuthSession {
            launcher_token,
            channel: payload.channel.clone(),
            channel_code,
            player_id,
            profile_name,
            profile_img_url,
            expires_at,
            device_key: device_key.to_string(),
            refresh_token: None,
        };

        Self::save_session(&session)?;
        println!("✓ ¡Sesión oficial de Netmarble guardada con éxito para {}!", session.profile_name);

        Ok(session)
    }

    /// Procesa cualquier token, URL de callback, deeplink o JSON recibido del sistema oficial NM
    pub async fn process_auth_result(raw_input: &str) -> Result<AuthSession> {
        let trimmed = raw_input.trim();
        let client = reqwest::Client::builder().build()?;

        let mut access_token = String::new();
        let mut refresh_token: Option<String> = None;
        let mut netmarble_id: Option<String> = None;

        // Caso 1: JSON payload
        if trimmed.starts_with('{') {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(t) = val.get("accessToken")
                    .or_else(|| val.get("access_token"))
                    .or_else(|| val.get("launcher_token"))
                    .and_then(|v| v.as_str())
                {
                    access_token = t.to_string();
                }
                if let Some(r) = val.get("refreshToken")
                    .or_else(|| val.get("refresh_token"))
                    .and_then(|v| v.as_str())
                {
                    refresh_token = Some(r.to_string());
                }
                if let Some(nid) = val.get("netmarbleId")
                    .or_else(|| val.get("netmarble_id"))
                    .and_then(|v| v.as_str())
                {
                    netmarble_id = Some(nid.to_string());
                }
            }
        }

        // Caso 2: URL con query string (nmmonster2://, http://, https://, /redirectLauncher?...) o query string directa
        if access_token.is_empty() {
            let qs = if let Some(pos) = trimmed.find('?') {
                &trimmed[pos + 1..]
            } else if trimmed.contains('=') && (trimmed.contains("accessToken") || trimmed.contains("access_token")) {
                trimmed
            } else {
                ""
            };

            let clean_qs = qs.split_whitespace().next().unwrap_or(qs);

            if !clean_qs.is_empty() {
                for pair in clean_qs.split('&') {
                    if let Some((k, v)) = pair.split_once('=') {
                        let clean_v = v.split('#').next().unwrap_or(v);
                        match k {
                            "accessToken" | "access_token" | "launcher_token" | "launcherToken" | "token" => access_token = clean_v.to_string(),
                            "refreshToken" | "refresh_token" => refresh_token = Some(clean_v.to_string()),
                            "netmarbleId" | "netmarble_id" => netmarble_id = Some(clean_v.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }

        // Caso 3: Token directo (string plano)
        if access_token.is_empty() {
            let t = trimmed;
            if !t.starts_with('/')
                && !t.starts_with("http")
                && !t.contains(' ')
                && !t.contains('?')
                && !t.contains('&')
                && t.len() >= 20
            {
                access_token = t.to_string();
            }
        }

        if access_token.is_empty() || access_token.starts_with('/') || access_token.len() < 20 {
            return Err(anyhow!("No se pudo extraer un token de acceso válido de Netmarble."));
        }

        // Consultar usuario real de Netmarble Members (netmarble-auth/user)
        let user_data = Self::fetch_netmarble_user(&client, &access_token).await.ok();

        // Consultar perfil de juego si existe personaje creado en monster2
        let game_profile = Self::fetch_game_profile(&client, &access_token).await.ok();

        let final_nid = netmarble_id
            .or_else(|| user_data.as_ref().and_then(|u| u.netmarble_id.clone()))
            .unwrap_or_default();

        let profile_name = game_profile
            .as_ref()
            .and_then(|p| p.profile_name.clone())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| user_data.as_ref().and_then(|u| u.nickname.clone()).filter(|s| !s.trim().is_empty()))
            .or_else(|| user_data.as_ref().and_then(|u| u.mail_address.clone()).filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| {
                if !final_nid.is_empty() {
                    format!("Cuenta Netmarble ({})", &final_nid[..final_nid.len().min(8)])
                } else {
                    "Cuenta Netmarble".to_string()
                }
            });

        let profile_img_url = game_profile
            .as_ref()
            .and_then(|p| p.profile_img_url.clone())
            .unwrap_or_default();

        let channel_code = user_data
            .as_ref()
            .and_then(|u| u.accessed_channel_code)
            .unwrap_or(20);

        let channel = match channel_code {
            1 => "facebook",
            12 => "twitter",
            20 => "email",
            26 => "apple",
            29 => "google",
            30 => "steam",
            31 => "epic",
            32 => "playstation",
            _ => "netmarble",
        }.to_string();

        let (jwt_sub, jwt_exp) = Self::decode_jwt_claims(&access_token);
        let player_id = if !final_nid.is_empty() { final_nid } else { jwt_sub };
        let expires_at = if jwt_exp > 0 {
            jwt_exp
        } else {
            chrono::Utc::now().timestamp() + (30 * 86400) // 30 días
        };

        let session = AuthSession {
            launcher_token: access_token,
            channel,
            channel_code,
            player_id,
            profile_name,
            profile_img_url,
            expires_at,
            device_key: Self::get_or_create_device_key(),
            refresh_token,
        };

        Self::save_session(&session)?;
        println!("✓ ¡Sesión oficial NM guardada con éxito para {}!", session.profile_name);

        Ok(session)
    }

    /// Permite procesar directamente un token de lanzador ya emitido o pegado manualmente
    #[allow(dead_code)]
    pub async fn direct_token_login(launcher_token: &str) -> Result<AuthSession> {
        Self::process_auth_result(launcher_token).await
    }

    /// Consulta datos del usuario oficial a Netmarble Members API (netmarble-auth/user)
    pub async fn fetch_netmarble_user(
        client: &reqwest::Client,
        access_token: &str,
    ) -> Result<NetmarbleUserData> {
        let url = format!("{}/netmarble-auth/user", NM_API_BASE);
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("X-NM-LAUNCHER-CH", NM_LAUNCHER_CH)
            .header("X-NM-LAUNCHER-VER", NM_LAUNCHER_VER)
            .send()
            .await?;

        let api_resp: NetmarbleApiResponse<NetmarbleUserData> = resp.json().await?;
        if api_resp.code == 0 && api_resp.data.is_some() {
            Ok(api_resp.data.unwrap())
        } else {
            Err(anyhow!(
                "No se pudo obtener datos de usuario: {}",
                api_resp.msg.unwrap_or_default()
            ))
        }
    }

    /// Consulta datos del perfil del jugador a Netmarble API (platformAuthType: NM)
    pub async fn fetch_game_profile(
        client: &reqwest::Client,
        launcher_token: &str,
    ) -> Result<GameProfileData> {
        let url = format!("{}/game-profile?gameCode={}&platformAuthType=NM", NM_API_BASE, GAME_CODE);
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", launcher_token))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("X-NM-LAUNCHER-CH", NM_LAUNCHER_CH)
            .header("X-NM-LAUNCHER-VER", NM_LAUNCHER_VER)
            .send()
            .await?;

        let api_resp: NetmarbleApiResponse<GameProfileData> = resp.json().await?;
        if api_resp.code == 0 && api_resp.data.is_some() {
            Ok(api_resp.data.unwrap())
        } else {
            Err(anyhow!("Sin personaje creado aún"))
        }
    }

    /// Decodifica los claims del JWT de Netmarble sin verificar la firma (obtiene sub y exp)
    fn decode_jwt_claims(jwt: &str) -> (String, i64) {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() < 2 {
            return (String::new(), 0);
        }

        use data_encoding::BASE64URL_NOPAD;
        let payload_b64 = parts[1];
        let decoded_bytes: Result<Vec<u8>, _> = BASE64URL_NOPAD.decode(payload_b64.as_bytes());

        if let Ok(bytes) = decoded_bytes {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let sub = val["sub"].as_str().unwrap_or_default().to_string();
                let exp = val["exp"].as_i64().unwrap_or(0);
                return (sub, exp);
            }
        }

        (String::new(), 0)
    }

    /// Construye la URL oficial de SSO de Netmarble Members (NM) para abrir en el navegador
    pub fn build_auth_url(channel: &str, port: u16, lang: &str) -> String {
        let language = match lang {
            "es" | "es_ES" | "es-ES" => "es",
            "en" | "en_US" | "en-US" => "en",
            "ko" | "ko_KR" => "ko",
            "ja" => "ja",
            "zh" => "zh",
            _ => "es",
        };

        let device_key = Self::get_or_create_device_key();
        let redirect_url = format!("http%3A%2F%2F127.0.0.1%3A{}%2FredirectLauncher", port);

        let idp_param = match channel {
            "google" => "&idpType=google",
            "apple" => "&idpType=apple",
            _ => "",
        };

        format!(
            "https://members.netmarble.com/auth?clientId={}&countryCode=US&language={}&osType=PC&webViewType=web&authType=signin&deviceKey={}_{}&showImageBanner=N&gameCode={}&forceCheckIdentity=N&redirectUrl={}&supportFeature=redirectParams%7CaccessToken{}",
            NM_CLIENT_ID, language, device_key, GAME_CODE, GAME_CODE, redirect_url, idp_param
        )
    }

    /// Inicia el servidor HTTP / WebSocket en 127.0.0.1:port para esperar el callback del navegador
    pub async fn listen_for_auth_callback(
        port: u16,
    ) -> Result<(u16, tokio::sync::oneshot::Receiver<AuthSession>)> {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(_) => {
                let fallback = TcpListener::bind("127.0.0.1:0").await?;
                fallback
            }
        };

        let bound_port = listener.local_addr()?.port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx_mutex = Arc::new(Mutex::new(Some(tx)));

        tokio::spawn(async move {
            println!("Servidor de autenticación local escuchando en http://127.0.0.1:{}/redirectLauncher y ws://", bound_port);

            // Timeout de 5 minutos para que el usuario complete el inicio de sesión
            let timeout_fut = tokio::time::sleep(std::time::Duration::from_secs(300));
            tokio::pin!(timeout_fut);

            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((mut stream, _peer)) => {
                                let tx_inner = tx_mutex.clone();
                                tokio::spawn(async move {
                                    use tokio::io::{AsyncReadExt, AsyncWriteExt};

                                    let mut peek_buf = [0u8; 2048];
                                    let peek_n = match stream.peek(&mut peek_buf).await {
                                        Ok(n) if n > 0 => n,
                                        _ => return,
                                    };

                                    let peek_str = String::from_utf8_lossy(&peek_buf[..peek_n]);
                                    let is_websocket = peek_str.to_ascii_lowercase().contains("upgrade: websocket");

                                    if is_websocket {
                                        if let Ok(mut ws_stream) = tokio_tungstenite::accept_async(stream).await {
                                            while let Some(msg_res) = ws_stream.next().await {
                                                if let Ok(msg) = msg_res {
                                                    if msg.is_text() {
                                                        let text = msg.to_text().unwrap_or_default();
                                                        println!("Mensaje recibido del navegador en WebSocket: {}", text);

                                                        if let Ok(session) = Self::process_auth_result(text).await {
                                                            let mut opt = tx_inner.lock().await;
                                                            if let Some(sender) = opt.take() {
                                                                let _ = sender.send(session);
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        // Petición HTTP directa de redirección
                                        let mut req_buf = vec![0u8; 8192];
                                        let read_n = match stream.read(&mut req_buf).await {
                                            Ok(n) if n > 0 => n,
                                            _ => return,
                                        };
                                        let req_str = String::from_utf8_lossy(&req_buf[..read_n]);

                                        let first_line = req_str.lines().next().unwrap_or_default();
                                        let target = first_line.split_whitespace().nth(1).unwrap_or_default();

                                        // Ignorar favicons y recursos secundarios solicitados automáticamente por navegadores
                                        if target.starts_with("/favicon") || target.ends_with(".ico") || target.ends_with(".png") {
                                            let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                            let _ = stream.write_all(not_found.as_bytes()).await;
                                            let _ = stream.flush().await;
                                            return;
                                        }

                                        let is_auth_request = target.contains("accessToken")
                                            || target.contains("access_token")
                                            || target.contains("launcher_token")
                                            || target.contains("launcherToken")
                                            || target.contains("token=")
                                            || target.contains("code=");

                                        if !target.starts_with("/redirectLauncher") && !is_auth_request {
                                            let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                            let _ = stream.write_all(not_found.as_bytes()).await;
                                            let _ = stream.flush().await;
                                            return;
                                        }

                                        let html_body = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Mongil: Star Dive - Autenticación</title></head>
<body style="background:#13151b;color:#e1e7ec;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;">
  <div style="background:#1e222d;padding:40px;border-radius:16px;box-shadow:0 8px 32px rgba(0,0,0,0.5);text-align:center;max-width:440px;border:1px solid #2d3345;">
    <h2 style="color:#00d26a;margin:0 0 12px;font-size:24px;">✓ ¡Inicio de sesión exitoso!</h2>
    <p style="color:#8a99ad;font-size:15px;margin:0 0 20px;line-height:1.5;">Tu cuenta de Netmarble se vinculó correctamente con Stardive Launcher.</p>
    <p style="color:#56627a;font-size:13px;margin:0;">Ya puedes cerrar esta pestaña y regresar al juego.</p>
  </div>
  <script>setTimeout(function(){ window.close(); }, 1500);</script>
</body>
</html>"#;

                                        let http_response = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                            html_body.len(),
                                            html_body
                                        );

                                        let _ = stream.write_all(http_response.as_bytes()).await;
                                        let _ = stream.flush().await;

                                        if is_auth_request {
                                            println!("Petición HTTP de autenticación recibida: {}", target);
                                            if let Ok(session) = Self::process_auth_result(target).await {
                                                let mut opt = tx_inner.lock().await;
                                                if let Some(sender) = opt.take() {
                                                    let _ = sender.send(session);
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                eprintln!("Error al aceptar conexión TCP: {}", e);
                                break;
                            }
                        }
                    }
                    _ = &mut timeout_fut => {
                        println!("Timeout alcanzado en el servidor de autenticación local.");
                        break;
                    }
                }
            }
        });

        Ok((bound_port, rx))
    }
}

#[cfg(test)]
mod tests {
    use super::AuthManager;

    /// El `deviceKey` (UUID del launcher) vive en `[Netmarble\\NetmarbleSDK]` → `NMDeviceKey`.
    #[test]
    fn parsea_launcher_device_key_real() {
        let reg = r#"
WINE REGISTRY Version 2
;; All keys relative to \\User\\S-1-5-21

[Software\\Netmarble\\NetmarbleSDK] 1789100489
#time=1dd41a504a2e832
"NMDeviceKey"="1bae41e6-171e-4e0f-a2b7-37ca48429520"

[Software\\Valve\\Steam] 1787951530
"SteamExe"="C:\\Program Files (x86)\\Steam\\Steam.exe"
"#;
        let got = AuthManager::parse_launcher_device_key_from_reg_content(reg);
        assert_eq!(got.as_deref(), Some("1bae41e6-171e-4e0f-a2b7-37ca48429520"));
    }

    /// El `nmDeviceKey` (MD5 del SDK) vive en `[Netmarble dev\\monster2]` → `DeviceKey`.
    #[test]
    fn parsea_nm_device_key_real() {
        let reg = r#"[Software\\Netmarble\\NetmarbleSDK]
"NMDeviceKey"="1bae41e6-171e-4e0f-a2b7-37ca48429520"

[Software\\Netmarble dev\\monster2] 1789100489
"DeviceKey"="C7760262485247B631ED50B16D02FA10"
"DeviceKeyForNetmarbleId"="D943FB2D4CED0E73939D0BB51C58C899"
"#;
        let got = AuthManager::parse_nm_device_key_from_reg_content(reg);
        assert_eq!(got.as_deref(), Some("C7760262485247B631ED50B16D02FA10"));
    }

    /// Variante con un solo backslash en la sección (por si algún backend
    /// serializa distinto al Wine real).
    #[test]
    fn parsea_variante_un_backslash() {
        // Un solo backslash literal entre cada nivel de la ruta.
        let reg = "[Software\\Netmarble\\NetmarbleSDK]\n\"NMDeviceKey\"=\"ABCDEF0123456789\"\n";
        let got = AuthManager::parse_launcher_device_key_from_reg_content(reg);
        assert_eq!(got.as_deref(), Some("ABCDEF0123456789"));
    }

    /// CRÍTICO: el parser del `nmDeviceKey` NO debe tomar la `DeviceKey` de la
    /// sección vecina `[Netmarble\\NetmarbleSDK]` (que tiene el UUID del launcher).
    /// Antes el parser sangraba entre secciones y devolvía el valor equivocado.
    #[test]
    fn nm_device_key_no_sangra_a_seccion_vecina() {
        let reg = r#"[Software\\Netmarble\\NetmarbleSDK]
#time=1dd41a504a2e832

[Software\\Netmarble dev\\monster2]
"DeviceKeyForNetmarbleId"="D943FB2D4CED0E73939D0BB51C58C899"
"#;
        // La sección `monster2` no tiene `DeviceKey` → None (NO debe devolver la de arriba).
        let got = AuthManager::parse_nm_device_key_from_reg_content(reg);
        assert_eq!(got, None);
    }

    /// El parser del deviceKey del launcher prefiere `NMDeviceKey` sobre `DeviceKey`
    /// dentro de la misma sección.
    #[test]
    fn prefiere_nmdevicekey_sobre_devicekey() {
        let reg = r#"[Software\\Netmarble\\NetmarbleSDK]
"DeviceKey"="DEADBEEF"
"NMDeviceKey"="CAFEBABE"
"#;
        let got = AuthManager::parse_launcher_device_key_from_reg_content(reg);
        assert_eq!(got.as_deref(), Some("CAFEBABE"));
    }

    /// Sin la sección → None (nunca inventar).
    #[test]
    fn sin_seccion_devuelve_none() {
        let reg = "[Software\\\\Valve\\\\Steam]\n\"Foo\"=\"Bar\"\n";
        assert_eq!(AuthManager::parse_launcher_device_key_from_reg_content(reg), None);
        assert_eq!(AuthManager::parse_nm_device_key_from_reg_content(reg), None);
    }

    #[test]
    fn build_auth_url_incluye_web_redirect_y_soporte_token() {
        let url = AuthManager::build_auth_url("google", 55000, "es");
        assert!(url.contains("webViewType=web"), "Debe usar webViewType=web para redirección estándar de navegador");
        assert!(url.contains("redirectUrl=http%3A%2F%2F127.0.0.1%3A55000%2FredirectLauncher"), "Debe tener redirectUrl a localhost");
        assert!(url.contains("supportFeature=redirectParams%7CaccessToken"), "Debe solicitar el accessToken en redirectParams");
        assert!(url.contains("idpType=google"), "Debe enviar idpType=google para Google SSO");
    }

    #[tokio::test]
    async fn process_auth_result_rechaza_favicon_y_rutas_http() {
        assert!(AuthManager::process_auth_result("/favicon.ico").await.is_err());
        assert!(AuthManager::process_auth_result("/redirectLauncher").await.is_err());
        assert!(AuthManager::process_auth_result("/").await.is_err());
        assert!(AuthManager::process_auth_result("").await.is_err());
    }

    #[test]
    fn save_session_rechaza_token_invalido() {
        let bad_session = super::AuthSession {
            launcher_token: "/favicon.ico".to_string(),
            channel: "email".to_string(),
            channel_code: 20,
            player_id: String::new(),
            profile_name: "Test".to_string(),
            profile_img_url: String::new(),
            expires_at: 0,
            device_key: "dev".to_string(),
            refresh_token: None,
        };
        assert!(AuthManager::save_session(&bad_session).is_err());
    }
}

