# Fase B — Diseño de red: Servidor / Cliente / Todo-aquí (LAN + túnel)

Estado: **diseño para aprobación**. No se escribió código de red.
Rama: `simplificacion/raices-fase-a`. Base medida: gateway `Server::http(("127.0.0.1", p))`
(`server.rs:387`), motor con host loopback fijo (`process.rs` `build_engine_cmd`),
auth `gateway.key` 64-hex (`auth.rs`), CORS solo-loopback (`meta.rs:177`).

## B0. Principios no negociables

1. **El motor (`llama-server`) jamás sale a la LAN.** Solo el gateway OMNI escucha
   fuera de loopback. El puerto crudo del motor queda en `127.0.0.1` siempre,
   con su `--api-key` (D-45) intacto. Doble frontera: LAN → gateway con clave →
   motor en loopback con clave.
2. **Loopback sigue siendo el default.** Exponer en LAN es opt-in explícito por
   arranque (`lan.enabled` persistente + confirmación en UI). Apagarlo vuelve al
   comportamiento actual byte por byte.
3. **Sin TLS casero, sin UPnP, sin internet directa.** `tiny_http` es thread-por-request
   sin rate-limit: exponerlo a internet pelado es imprudente. Internet solo vía
   túnel asistido (el operador del túnel termina TLS y autentica el borde).
4. **La clave existente se reutiliza, no se inventa otra.** `gateway.key` ya tiene
   64 hex y tres vías (Bearer / `x-api-key` / cookie). El pairing la distribuye,
   no la reemplaza.

## B1. Modos

| Modo | Gateway bind | Motor local | UI |
|---|---|---|---|
| Todo aquí (default) | `127.0.0.1` | sí | WebView directa, sin cambios |
| Servidor | `0.0.0.0:<puerto>` opt-in | sí | local directa + remotos con clave |
| Cliente | `127.0.0.1` | no (apagado e inarrancable) | chat proxyeado al remoto |

`Cliente` bloquea `/api/start` con 409 (`{"error":"modo_cliente_sin_motor"}`) para que
ningún cliente viejo arranque cómputo donde no debe.

## B2. Regla del peer (núcleo del diseño)

`tiny_http::Request::remote_addr()` en cada request, ANTES de auth:

- Peer loopback (`127.0.0.1`, `::1`) → siempre permitido (WebView, CLIs, túnel local).
- Peer RFC1918 (`10/8`, `172.16/12`, `192.168/16`) → permitido **solo si `lan.enabled`**,
  si no 403 + log.
- Cualquier otro peer → 403 + log, siempre (aunque `lan.enabled`).

Pura y testeable: `peer_permitido(ip: IpAddr, lan_enabled: bool) -> bool`.
Esto deja fuera internet directa por construcción, incluso con el socket en `0.0.0.0`.

## B3. CORS por allowlist (evolución de P29)

Reflejar `Origin` solo si:

- es loopback (regla actual, intacta), o
- `lan.enabled` y el host del Origin es RFC1918 o `localhost`.

Nada de `*`. Función pura `origen_permitido(origin, lan_enabled)`, con los tests
de `is_loopback_origin` intactos más casos LAN.

## B4. UI pública en LAN: con desbloqueo

Hoy `/`, `/index.html` e iconos son públicos y **fijan cookie `lm_key`**
(`server.rs:469-509`): en LAN eso autenticaría a cualquier visitante.
Regla nueva:

- Desde loopback: igual que hoy (la ventana local entra directa).
- Desde LAN con `lan.enabled`: `/` sirve pantalla de desbloqueo (campo clave,
  nada de telemetría ni endpoints hasta autenticar). Las `/api/*` ya exigen clave.
- Con `lan.enabled=false`: la UI en `0.0.0.0` ni siquiera existe (bind loopback).

## B5. Pairing (cómo la clave llega a la otra PC)

Opción recomendada **QR simple**:

- `GET /api/lan` (loopback o autenticado) devuelve `{ ips_rfc1918[], puerto }`.
- La UI Servidor muestra QR con payload texto:
  `omni://<ip>:<puerto>#k=<gateway.key>` más botón copiar y botón **regenerar clave**
  (rota `gateway.key`, invalida clientes viejos, reinicia motor con la nueva `--api-key`).
- El Cliente escanea/pega `URL + clave`, `POST /api/remote/test` verifica
  (GET remoto `/v1/models` con Bearer, timeout 8 s) y guarda en TOML
  (`[remote] url, key`). La UI nunca muestra la clave completa salvo enmascarada.

Alternativa descartada por defecto: token de un solo uso canjeable (más piezas,
mismo nivel de seguridad en LAN hogareña confiable). Se adopta solo si el dueño
la pide (ver preguntas al final).

## B6. Modo Cliente: proxy de chat al remoto

- `POST /v1/chat/completions` (y `/v1/models`) en Cliente se reenvían al
  `remote.url` con `Authorization: Bearer <remote.key>`, streaming SSE intacto.
- La clave remota vive solo en el TOML del cliente y en memoria del proceso;
  jamás en logs, URLs ni respuestas.
- Validación de `remote.url`: esquema `http/https`, host IP-literale, RFC1918,
  `localhost`, o dominio del túnel declarado; nada de credenciales en la URL.
- Telemetría `usage.jsonl`: el Servidor registra (es quien computa); el Cliente
  registra `retransmitido_a` sin tokens (no duplica contabilidad).

## B7. Túnel para internet (asistido, no embebido)

OMNI **no** embebe ni ejecuta binarios de terceros y **no** abre puertos del router.
La UI Servidor guía dos caminos (el dueño elige el primero a documentar):

1. **Tailscale** (recomendado): red privada entre tus máquinas, sin abrir nada;
   el Cliente apunta a la IP Tailscale (entra por la regla RFC1918/CGNAT… ver
   nota) o al MagicDNS.
2. **Cloudflared** (`cloudflared tunnel --url http://127.0.0.1:<puerto>`): URL
   pública con TLS y Access de Cloudflare; el Cliente valida dominio `https`.

Nota: Tailscale usa CGNAT `100.64/10` (no RFC1918). La regla B2 debe aceptar
`100.64/10` **solo si `lan.enabled`** (misma confianza que LAN: red privada del
dueño). Queda explícito y testeado, no como excepción silenciosa.

## B8. Firewall y packaging

- `packaging/` suma `firewall-lan.ps1`: crea regla inbound privada TCP al puerto
  gateway vigente (lee el TOML), solo perfil `Private`, con `-Remove` simétrico.
- Nunca perfil `Public`. Nunca regla al puerto del motor.

## B9. Superficie nueva (contrato)

Config (`[lan] enabled=false`, `[remote] url="", key=""`, `[client_mode] enabled=false`):

- `GET /api/lan` → `{ enabled, ips[], port }` (loopback o con clave).
- `POST /api/lan { enabled: bool }` → cambia bind **con reinicio del gateway**
  (el socket se liga al arrancar; sin hot-swap en v1).
- `POST /api/remote/test { url, key }` → `{ ok, models[] }` o `{"error": ...}`.
- `POST /api/remote { url, key }` → persiste (clave nunca vuelve en GET).
- `POST /api/key/rotate` → regenera `gateway.key` (loopback o con clave vieja
  válida; invalida sesiones).

## B10. Puertas y reversión

- Unitarios nuevos: `peer_permitido`, `origen_permitido`, validación `remote.url`,
  pairing payload/rotate. `cargo test` verde + clippy sin nuevos warnings.
- E2E en una sola máquina: dos instancias (puertos distintos), una en modo
  Servidor-LAN y otra Cliente apuntando a `127.0.0.1`… limitada porque loopback
  siempre pasa: el caso RFC1918 se cubre con tests de la regla pura + prueba
  manual documentada entre dos PCs.
- Reversión: `lan.enabled=false` + modo Todo-aquí = árbol actual. Ningún cambio
  de formato en `/api/*` ni `/v1/*` existentes; SRS §1.2/§2.5 se actualiza
  (la prohibición "no LAN" se levanta y la sustituye este documento).

## B11. Lo que este diseño NO hace (a propósito)

- Sin mDNS/anuncio automático (otra dependencia y otro servicio; el QR basta en v1).
- Sin multi-usuario ni cuotas (un secreto compartido por hogar; documentado).
- Sin hot-swap del bind (reinicio del gateway al cambiar `lan.enabled`).
- Sin exponer `/metrics`, `/api/logs` ni `/api/config` más allá de lo que la clave
  ya protege: con clave válida en LAN se ve lo mismo que en local. Si el dueño
  quiere rol "invitado solo-chat", es Fase B2, no v1.
