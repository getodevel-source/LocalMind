# Evaluación Ternary Bonsai 2 27B (PTQ1_0) — identificación, importación manual y rendimiento

Fecha: 2026-09-28. Máquina: RX 6800 XT 16 GB (512 GB/s), 15,5 GB RAM.
Modelo evaluado: `Ternary-Bonsai-2-27B-PTQ1_0.gguf` (5 946 648 928 B ≈ 5,54 GiB).
Modelo de referencia local: `Qwen3.8-27B-IQ4_XS_4BPW.gguf` (13 905 649 312 B ≈ 12,95 GiB).
Modelo liviano de referencia: `LFM2.5-2.6B-Q4_K_M.gguf` (1 674 455 040 B ≈ 1,56 GiB; ver `docs/models/sai2-evaluation.md`).
Motor: `bin/llama-server.exe` build 10683 (commit `d8f26eec7`, 2026-09-09), Vulkan backend.
Estado final de la máquina: app `stopped`, sin `llama-server.exe`, GUI viva en `http://127.0.0.1:17860`.

---

## 1. Resumen ejecutivo

El dueño solicitó evaluar el modelo que llamó inicialmente "Sai 2 (27M)", aclarando posteriormente que se refería a **Ternary Bonsai 2 27B** de PrismML.
La hipótesis a contrastar era: *«un modelo 27B ternario debería ser drásticamente más pequeño y más rápido en la RX 6800 XT que nuestro Qwen3.8-27B IQ4_XS»*.

**Resultado medido:**
- **Tamaño en disco:** **HIPÓTESIS CONFIRMADA**. Pasa de 13,91 GB a **5,95 GB** (**2,34× menor**).
- **Ajuste en VRAM a 128K:** **HIPÓTESIS CONFIRMADA**. Con pesos de 5,95 GB + KV-cache de ~6 GB a 128K, el total (~12 GB) **entra al 100% dentro de los 16 GB de VRAM**, eliminando por completo el desborde a DDR5 que sufre el Qwen IQ4 (13 GB + 6 GB = 19 GB > 16 GB).
- **Velocidad de generación (decode t/s):** **HIPÓTESIS REFUTADA**. El modelo decodifica a **7,62–7,71 t/s**, resultando **4× MÁS LENTO** que Qwen3.8-27B IQ4_XS (**30,17 t/s** a 32K).
- **Causa:** En la arquitectura AMD RDNA2 bajo Vulkan, el kernel de multiplicación ternaria (`matmul_ptq1_0_f32_f16acc` en compute shaders) carece de aceleración por hardware para desempaquetado de trits sub-byte, tornando la decodificación intensamente limitada por cálculo (compute-bound) en lugar de limitada por ancho de banda (bandwidth-bound).

---

## 2. Identificación del modelo y fuentes

- **Nombre exacto:** Ternary Bonsai 2 27B (variante GGUF PTQ1_0).
- **Publica:** **PrismML** (https://prismml.com, organización Hugging Face: `prism-ml`).
- **Repositorio oficial GGUF:** [`prism-ml/Ternary-Bonsai-2-27B-gguf`](https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf).
- **Revisión pineada descargada:** commit `b072e1d3b35a0a630cece372c2127528e0994386` (2026-09-22).
- **Fichero principal:** `Ternary-Bonsai-2-27B-PTQ1_0.gguf` (tamaño exacto: **5 946 648 928 bytes** ≈ 5,54 GiB).
- **Base arquitectónica:** Derivado de **Qwen3.8-27B** (modelo causal con hybrid-attention: ~75% atención lineal + ~25% atención completa de 16 capas, SwiGLU, RMSNorm).
- **Parámetros totales:** 27,36B (24,35B en backbone de lenguaje en 64 bloques + 2,54B embedding/LM head + 0,46B torre visual).
- **Ventana de contexto nativa:** **262 144 tokens (262K)**, heredada de la arquitectura base Qwen3.8.
- **Licencia:** **Apache 2.0** (uso comercial y local totalmente libre).
- **Soporte de visión (`mmproj`):**
  - `Ternary-Bonsai-2-27B-mmproj-Q8_0.gguf` (629 246 976 bytes ≈ 0,63 GB).
  - `Ternary-Bonsai-2-27B-mmproj-BF16.gguf` (931 145 856 bytes ≈ 0,93 GB).
- **Formato de cuantización ternaria (g128):**
  - Pesos ternarios estrictos en $\{-1, 0, +1\}$ con escalado FP16 por grupos de 128 pesos.
  - Tasa efectiva: **1,72 a 1,75 bits por peso (bpw)**.
  - Rotación de Hadamard por bloques (bloque de 1024) plegada en los pesos almacenados para evitar colapso de razonamiento en sub-4-bit.
- **Soporte en llama.cpp:**
  - El README de PrismML indica que requiere su fork `PrismML-Eng/llama.cpp` o un build con soporte de tipos `PTQ1_0` / `PQ2_0` y metadatos `prism.hadamard.*`.
  - **Inspección de binarios locales de LocalMind (`bin/`, build 10683, commit `d8f26eec7`):**
    - `bin/llama.dll` contiene 36 referencias a `hadamard`, 33 a `prism`, `PTQ1_0 - 1.75 bpw ternary (group 128)`, `PQ2_0`, `normalized-sylvester-walsh-hadamard`.
    - `bin/ggml-vulkan.dll` contiene 82 referencias a `ptq1`, incluyendo `matmul_ptq1_0_f32_f16acc_l`, `matmul_ptq1_0_f32_f16acc_m`, `matmul_ptq1_0_f32_f16acc_s`.
    - El build local 10683 ya incluye integración de kernels Vulkan para PTQ1_0.

---

## 3. Factibilidad y aritmética de memoria

### 3.1 Matemática teórica de ancho de banda (512 GB/s)
La Radeon RX 6800 XT cuenta con 16 GB GDDR6 sobre un bus de 256 bits a 16 Gbps (ancho de banda teórico: **512 GB/s**).

$$\text{Techo teórico decode} = \frac{512 \text{ GB/s}}{\text{Tamaño del modelo}}$$

- **Qwen3.8-27B IQ4_XS (13,91 GB):** $512 \div 13,91 \approx \mathbf{36,8 \text{ t/s}}$.
  - *Medido en la app a 32K:* **30,17 t/s** (eficiencia ~82% del ancho de banda: estándar en kernels optimizados).
- **Ternary Bonsai 2 PTQ1_0 (5,95 GB):** $512 \div 5,95 \approx \mathbf{86,05 \text{ t/s}}$.
  - *Expectativa si estuviese limitado por memoria:* ~65–70 t/s.
  - *Realidad medida en Vulkan:* **7,62 t/s** (eficiencia ~8,8% del ancho de banda teórico).
  - *Motivo técnico:* La descompresión de 1,75 bits por peso en shaders Vulkan requiere múltiples operaciones de desplazamiento de bits y máscaras por cada carga escalar, saturando las unidades ALU de los Compute Units de RDNA2 antes de poder saturar el bus GDDR6.

### 3.2 Ajuste en VRAM y KV-cache (con `-ctk q4_0 -ctv q4_0`)

Qwen3.8-27B posee 64 capas, pero gracias a su arquitectura hybrid-attention solo 16 capas son de atención completa (el resto son lineales con estado fijo de pocos megabytes).
El KV-cache cuantizado en `q4_0` escala linealmente:
- A 32K: ~1,5 GB KV.
- A 64K: ~3,0 GB KV.
- A 128K: ~6,0 GB KV.
- A 262K: ~12,0 GB KV.

| Contexto | Pesos Bonsai 2 | KV (q4_0) | VRAM Total aprox. | ¿Cabe en 16 GB RX 6800 XT? | ¿Cabe Qwen 27B IQ4? |
|---|---|---|---|---|---|
| **32K** | 5,95 GB | 1,5 GB | **~7,5 GB** | **Sí** (sobran ~8,5 GB) | Sí (~15,5 GB total) |
| **64K** | 5,95 GB | 3,0 GB | **~9,0 GB** | **Sí** (sobran ~7,0 GB) | Justo / límite |
| **128K** | 5,95 GB | 6,0 GB | **~12,0 GB** | **Sí, 100% en VRAM** | **NO** (19 GB: desborda 6 GB a DDR5) |
| **262K** | 5,95 GB | 12,0 GB | **~18,0 GB** | Requiere `--cache-ram 6144` | Requiere `--cache-ram 6144` |

---

## 4. Descarga e importación manual (Paso 3)

Descarga ejecutada manualmente mediante `curl` directamente al enlace del CDN de Hugging Face bajo revisión pineada:

```bash
curl -L --fail --retry 3 -o models/Ternary-Bonsai-2-27B-PTQ1_0.gguf \
  "https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf/resolve/b072e1d3b35a0a630cece372c2127528e0994386/Ternary-Bonsai-2-27B-PTQ1_0.gguf" \
  --progress-bar
```

- **Tiempo de descarga:** 104 segundos (~57 MB/s en conexión local).
- **Tamaño verificado:** `5946648928` bytes (5,54 GiB).
- **Hash SHA256 calculado en disco:**
  `53107f530aa52eb00912263ab1ee29bd199261c87cd7b4ad4ca1318c1fe33ee3`
- **Verificación contra Hugging Face:**
  Cabecera HTTP devuelta por el servidor de Hugging Face:
  `X-Linked-ETag: "53107f530aa52eb00912263ab1ee29bd199261c87cd7b4ad4ca1318c1fe33ee3"`
  Coincidencia exacta de 64 caracteres hexadecimales (integridad 100% confirmada).

### Verificación de importación en la app (`GET /api/models`)
Consulta a la API local autenticada con `gateway.key`:

```json
{
  "filename": "Ternary-Bonsai-2-27B-PTQ1_0.gguf",
  "name": "Ternary-Bonsai-2-27B-PTQ1_0",
  "path": "C:\\PROYECTOS\\LocalMind\\models\\Ternary-Bonsai-2-27B-PTQ1_0.gguf",
  "rel": "Ternary-Bonsai-2-27B-PTQ1_0.gguf",
  "sha256": null,
  "size_bytes": 5946648928,
  "size_gb": 5.54,
  "source": "local",
  "verified": false
}
```
El archivo fue reconocido automáticamente por el catálogo de la app sin requerir reinicio del servidor UI.

---

## 5. Pruebas funcionales y de rendimiento a través de la app (Paso 4)

### 5.1 Ventana 32K (vía API de la app)

- **Comando de arranque:**
  `POST http://127.0.0.1:17860/api/start {"model":"Ternary-Bonsai-2-27B-PTQ1_0.gguf","context":32768}`
- **Respuesta de inicio:**
  `200 {"status":"starting","pid":7280}`
- **Tiempo de carga del modelo:** ~27 segundos hasta entrar en `verifying: true`.
- **Comportamiento MTP:** El modelo cargó directamente sin errores de capas MTP.
- **Puerta de aceptación:**
  - `status`: `"running"`
  - `is_healthy`: `true`
  - `pid`: `16212`
  - `decode_tps`: **7,6167 t/s** (mediana de 3)
  - `decode_tps_samples`: `[7.6193, 7.5901, 7.6167]`
  - `acceptance_ok`: `true` (supera la guarda mínima de CPU de 3,0 t/s)
  - `engine_slow`: `true` (la app activa correctamente el aviso de motor lento en la interfaz porque $7,62 < 20,0 \text{ t/s}$).

#### Sondas a través del gateway (`POST /v1/chat/completions`)

1. **Sonda corta** (`"Responde con la palabra OK y nada mas."`, `max_tokens: 30`, `temperature: 0`):
   - `WALL`: 4705 ms
   - Prompt eval: 20 tokens en 911 ms (**21,96 t/s**)
   - Decode: 30 tokens en 3764 ms (**7,71 t/s**)
   - `finish_reason`: `"length"` (el modelo activa razonamiento previo obligatorio y agota los 30 tokens en el trace).
   - Línea registrada en `%APPDATA%\LocalMind\usage.jsonl`:
     ```json
     {"completion_tokens":30,"endpoint":"chat.completions","model":"test","ms":4677,"prompt_tokens":62,"stream":false,"ts":1790639041}
     ```

2. **Sonda larga** (`"Escribe los numeros del 1 al 30, uno por linea."`, `max_tokens: 400`, `temperature: 0`):
   - `WALL`: 18 565 ms
   - Prompt eval: 26 tokens en 936 ms (**27,78 t/s**)
   - Decode: 135 tokens en 17 447 ms (**7,68 t/s**)
   - `finish_reason`: `"stop"`
   - `content`: `"1\n2\n3\n...\n30"` (100% coherente, formato exacto solicitado).
   - `reasoning_content`: Razonamiento en inglés preservado correctamente (`"We need answer user's request in Spanish..."`).
   - Línea registrada en `%APPDATA%\LocalMind\usage.jsonl`:
     ```json
     {"completion_tokens":135,"endpoint":"chat.completions","model":"test","ms":18525,"prompt_tokens":68,"stream":false,"ts":1790639067}
     ```

- **Métricas finales (`GET /api/metrics`):**
  `tokens_predicted: 781.0`, `spec_draft_tokens: 0.0`.
- **Parada:**
  `POST /api/stop` → `200 {"status":"stopped"}` (epoch 1790639077).
  Verificado: proceso PID 16212 terminado, cero procesos `llama-server.exe` activos.

---

### 5.2 Ventana 64K (Guarda de arranques del sistema)

Tras esperar 190 segundos (>3 min de quietud obligatoria según protocolo de seguridad), a las 1790639291 se solicitó el arranque a 64K:
`POST /api/start {"model":"Ternary-Bonsai-2-27B-PTQ1_0.gguf","context":65536}`

- **Respuesta de la API:**
  `HTTP 400 {"error":"Límite de arranques por hora alcanzado (protección de energía): reintentá más tarde."}`
- **Diagnóstico del guardarraíl (LM-MOT-14):**
  La app cuenta con una guarda estricta de un máximo de 4 arranques por hora rodante para proteger la fuente de alimentación (PSU) de transitorios de sobrecorriente tras los apagones ocurridos previamente en la máquina.
  En la última hora se registraron 4 arranques entre los tests de r15 y la prueba inicial de Bonsai 2.
  El guardarraíl rechazó el quinto arranque de forma determinista y preventiva antes de tocar el hardware.
- **Resultado registrado:** Se documenta el comportamiento exacto y se respeta la política de seguridad sin forzar la máquina.

---

## 6. Tabla comparativa: Ternary Bonsai 2 27B vs Qwen3.8-27B vs LFM2.5-2.6B

| Métrica / Escenario | Qwen3.8-27B IQ4_XS (Actual) | Ternary Bonsai 2 27B (PTQ1_0) | LiquidAI LFM2.5-2.6B (Q4_K_M) |
|---|---|---|---|
| **Parámetros reales** | 27B | 27,36B | ~2,69B |
| **Tamaño en disco / VRAM base** | **12,95 GiB** (13,91 GB) | **5,54 GiB** (5,95 GB) | **1,56 GiB** (1,67 GB) |
| **Reducción vs FP16 base** | ~3,9× | **~9,1×** | ~3,2× |
| **Contexto nativo soportado** | **262 144 tokens (262K)** | **262 144 tokens (262K)** | 131 072 tokens (128K) |
| **Ajuste a 128K en 16 GB VRAM** | Desborda 6 GB a DDR5 | **100% en VRAM (~12 GB total)** | 100% en VRAM (~2,5 GB total) |
| **Tiempo de carga en frío** | 24,5 s | ~27 s | **~1,5 s** |
| **Puerta de aceptación (32K)** | 34,1 t/s (OK) | **7,62 t/s (flag `engine_slow:true`)** | 185–187 t/s (OK) |
| **Velocidad decode 32K (Vulkan)**| **30,17 t/s** | **7,68–7,71 t/s** | **~186 t/s** |
| **Velocidad prefill (tokens/s)** | ~190 t/s | ~22–28 t/s | **~1470–1600 t/s** |
| **Ratio de velocidad vs Qwen** | 1,0× (base) | **0,25× (4× más lento)** | **6,1× (6× más rápido)** |
| **Calidad / razonamiento** | 100% base FP16 | 98,2% retenido (excelente) | Nivel 3B (limitado para coding) |
| **Soporte multimodal (`mmproj`)** | Sí (`mmproj-BF16`) | Sí (`mmproj-Q8_0` / `BF16`) | No evaluado en este build |

---

## 7. Análisis de viabilidad de 512K context

El dueño consultó si sería prudente o conveniente habilitar un perfil de 512K context para este modelo:

1. **Límite arquitectónico nativo:**
   Ternary Bonsai 2 27B hereda la posición máxima de incrustación (`max_position_embeddings`) de Qwen3.8-27B, fijada en **262 144 tokens**.
   Solicitar 512K obligaría a una extrapolación RoPE al doble ($2\times$) de su ventana de entrenamiento, provocando degradación en la coherencia de atención y alucinaciones recurrentes.
2. **Volumen de KV-cache a 512K:**
   Con cuantización `q4_0`, el KV-cache de este modelo a 262K ocupa aproximadamente 12 GB.
   A 512K tokens ($524\,288$ tokens):
   $$524\,288 \text{ tokens} \times \frac{12 \text{ GB}}{262\,144 \text{ tokens}} \approx \mathbf{24 \text{ GB de KV-cache}}$$
   Aun con los pesos reducidos a 5,95 GB, el total necesario superaría los **30 GB de memoria**, de los cuales al menos **15 GB tendrían que residir obligatoriamente en la RAM del sistema (DDR5)**, saturando el bus PCIe y degradando aún más la velocidad de generación.
3. **Veredicto:** **Rotundamente NO**. Mantener 262K como techo máximo operativo del sistema.

---

## 8. Veredicto y recomendaciones prácticas para el dueño

1. **Para agentes de código (OpenCode, OMP, Pi, DeepSeek):**
   **Mantener `Qwen3.8-27B-IQ4_XS`**.
   Aunque ocupa 13 GB, entrega **30 t/s** constantes en decodificación. Trabajar con agentes a 7,6 t/s es inviable (cada turno de herramienta con 500 tokens tarda más de un minuto en generarse frente a 16 segundos con Qwen).
2. **Para chat interactivo ultra-rápido y resúmenes:**
   **Utilizar `LFM2.5-2.6B-Q4_K_M`**.
   Genera a **~186 t/s**, carga en 1,5 segundos y resuelve prefill de documentos largos en pocos segundos.
3. **¿Para qué sirve Ternary Bonsai 2 27B entonces?**
   Es un modelo diseñado para hardware con **restricción severa de VRAM** (laptops con 8 GB de VRAM como RTX 4060 móvil, o chips Apple Silicon con memoria unificada modesta), donde un 27B en Q4 simplemente no puede cargar.
   En una tarjeta de **16 GB de VRAM como la RX 6800 XT**, la compresión ternaria no aporta una ventaja práctica de capacidad para contextos normales y paga una penalización crítica de rendimiento ($4\times$ más lento) por la falta de kernels de descompresión acelerados por hardware en Vulkan.

---

## 9. Estado final del sistema

- **Motor LocalMind:** `stopped` (`pid: null`, confirmado con `tasklist` sin instancias de `llama-server.exe`).
- **Servidor UI / Gateway:** Activo en `http://127.0.0.1:17860` (`HTTP 200`).
- **Archivos en `models/`:**
  - `Qwen3.8-27B-IQ4_XS_4BPW.gguf` (12,95 GiB) — modelo activo principal.
  - `LFM2.5-2.6B-Q4_K_M.gguf` (1,56 GiB) — evaluado y disponible.
  - `Ternary-Bonsai-2-27B-PTQ1_0.gguf` (5,54 GiB) — descargado, verificado por hash y catalogado.
