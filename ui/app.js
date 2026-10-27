"use strict";
/* ------------------------------------------------------------ icone (disegnate a mano) */
const P = (d, extra = "") => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" ${extra}>${d}</svg>`;
const I = {
  casa: P('<path d="M3.5 10.5 12 4l8.5 6.5"/><path d="M5.5 9v10.5h13V9"/><path d="M10 19.5v-5h4v5"/>'),
  schermo: P('<rect x="3" y="4.5" width="18" height="12" rx="2.5"/><path d="M8.5 20h7"/><path d="M12 16.5V20"/>'),
  frecce: P('<path d="M7 4v14"/><path d="m3.5 14.5 3.5 3.5 3.5-3.5"/><path d="M17 20V6"/><path d="m13.5 9.5 3.5-3.5 3.5 3.5"/>'),
  telefono: P('<rect x="6.5" y="2.5" width="11" height="19" rx="3"/><path d="M10.5 5.5h3"/>'),
  ingranaggio: P('<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M21.2 12h-2.4M5.2 12H2.8M18.5 5.5l-1.7 1.7M7.2 16.8l-1.7 1.7M18.5 18.5l-1.7-1.7M7.2 7.2 5.5 5.5"/>'),
  su: P('<path d="M12 15.5V4"/><path d="m7.5 8.5 4.5-4.5 4.5 4.5"/><path d="M4.5 14.5v3a2.5 2.5 0 0 0 2.5 2.5h10a2.5 2.5 0 0 0 2.5-2.5v-3"/>'),
  invia: P('<path d="M4 12 20 4l-6 16-3-7z"/><path d="m11 13 9-9"/>'),
  appunti: P('<rect x="6" y="4.5" width="12" height="16" rx="2.5"/><path d="M9.5 3.5h5v3h-5z"/>'),
  copia: P('<rect x="8.5" y="8.5" width="11" height="11" rx="2.5"/><path d="M15.5 8.5V6.5A2 2 0 0 0 13.5 4.5h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>'),
  play: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z"/></svg>',
  stop: '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6.5" y="6.5" width="11" height="11" rx="2.5"/></svg>',
  mirror: P('<rect x="3" y="7" width="12" height="9" rx="2"/><path d="M9 4h10a2 2 0 0 1 2 2v7"/>'),
  foto: P('<rect x="3" y="4.5" width="18" height="15" rx="3"/><circle cx="9" cy="10" r="1.8"/><path d="m21 16-5-5-8.5 8.5"/>'),
  video: P('<rect x="3" y="6" width="13" height="12" rx="2.5"/><path d="m16 10.5 5-3v9l-5-3"/>'),
  audio: P('<path d="M9 18V6l11-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>'),
  pdf: P('<path d="M14 3.5H7A2.5 2.5 0 0 0 4.5 6v12A2.5 2.5 0 0 0 7 20.5h10a2.5 2.5 0 0 0 2.5-2.5V9z"/><path d="M14 3.5V9h5.5"/><path d="M8.5 14h7M8.5 17h4.5"/>'),
  file: P('<path d="M14 3.5H7A2.5 2.5 0 0 0 4.5 6v12A2.5 2.5 0 0 0 7 20.5h10a2.5 2.5 0 0 0 2.5-2.5V9z"/><path d="M14 3.5V9h5.5"/>'),
  testo: P('<path d="M5 6.5h14M5 11h14M5 15.5h9"/>'),
  archivio: P('<rect x="3.5" y="4.5" width="17" height="5" rx="1.5"/><path d="M5 9.5v8A2.5 2.5 0 0 0 7.5 20h9a2.5 2.5 0 0 0 2.5-2.5v-8"/><path d="M10 13h4"/>'),
  cartella: P('<path d="M3.5 7.5A2.5 2.5 0 0 1 6 5h3.5l2 2.5H18a2.5 2.5 0 0 1 2.5 2.5v7.5A2.5 2.5 0 0 1 18 20H6a2.5 2.5 0 0 1-2.5-2.5z"/>'),
  x: P('<path d="M6 6l12 12M18 6 6 18"/>'),
  ok: P('<path d="m5 12.5 4.5 4.5L19 7.5"/>'),
  attenzione: P('<path d="M12 4 2.8 19.5h18.4z"/><path d="M12 10v4.5M12 17.2v.1"/>'),
  pc: P('<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M2 19.5h20"/>'),
  luna: P('<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>'),
  goccia: P('<path d="M12 3.5s6.5 7 6.5 11a6.5 6.5 0 0 1-13 0c0-4 6.5-11 6.5-11z"/>'),
  scudo: P('<path d="M12 3.5 5 6v5.5c0 4.5 3 7.7 7 9 4-1.3 7-4.5 7-9V6z"/><path d="m9 12 2 2 4-4"/>'),
  campana: P('<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>'),
  accendi: P('<path d="M12 3.5v8"/><path d="M7 6.5a7 7 0 1 0 10 0"/>'),
  wifi: P('<path d="M2.5 9a14 14 0 0 1 19 0"/><path d="M5.5 12.5a9.5 9.5 0 0 1 13 0"/><path d="M8.7 16a5 5 0 0 1 6.6 0"/><circle cx="12" cy="19.2" r=".9" fill="currentColor"/>'),
  cavo: P('<path d="M9 3.5v4M15 3.5v4"/><rect x="6.5" y="7.5" width="11" height="6" rx="2"/><path d="M12 13.5v3a3 3 0 0 1-3 3H7"/>'),
  lucchetto: P('<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8.5 10.5V7.5a3.5 3.5 0 0 1 7 0v3"/>'),
  occhio: P('<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>'),
  messaggio: P('<path d="M20.5 11.5c0 4.1-3.8 7.5-8.5 7.5-1.2 0-2.3-.2-3.3-.6L4 20l1.3-3.6A7 7 0 0 1 3.5 11.5C3.5 7.4 7.3 4 12 4s8.5 3.4 8.5 7.5z"/>'),
  rete: P('<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.5 2.5 3.5 5.5 3.5 8.5s-1 6-3.5 8.5c-2.5-2.5-3.5-5.5-3.5-8.5s1-6 3.5-8.5z"/>'),
  stella: P('<path d="m12 3.8 2.5 5.2 5.7.8-4.1 4 1 5.7-5.1-2.7-5.1 2.7 1-5.7-4.1-4 5.7-.8z"/>'),
  stellaPiena: '<svg viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="m12 3.8 2.5 5.2 5.7.8-4.1 4 1 5.7-5.1-2.7-5.1 2.7 1-5.7-4.1-4 5.7-.8z"/></svg>',
  mondo: P('<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17"/>'),
};
function icone(root = document) { root.querySelectorAll("[data-i]").forEach(e => { if (!e.dataset.done) { e.innerHTML = I[e.dataset.i] || ""; e.dataset.done = 1; } }); }

/* ------------------------------------------------------------ utilità */
const $ = s => document.querySelector(s);
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const dim = n => { n = +n || 0; const u = ["B", "KB", "MB", "GB", "TB"]; let i = 0; while (n >= 1024 && i < 4) { n /= 1024; i++; } return (i ? n.toFixed(n < 10 ? 1 : 0) : n) + " " + u[i]; };
const vel = n => n ? " · " + dim(n) + "/s" : "";
const fa = t => { const s = Date.now() / 1000 - t; if (s < 60) return "adesso"; if (s < 3600) return Math.floor(s / 60) + " min fa"; if (s < 86400) return Math.floor(s / 3600) + " h fa";
  const d = new Date(t * 1000); return d.toLocaleDateString("it-IT", { day: "numeric", month: "short" }); };
const iniziale = s => (String(s || "?").trim()[0] || "?").toUpperCase();
const dur = s => s < 60 ? Math.max(1, Math.round(s)) + " s" : s < 3600 ? Math.round(s / 60) + " min" : Math.floor(s / 3600) + " h " + Math.round(s % 3600 / 60) + " min";
/* nome da mostrare: l'etichetta del motore distingue i dispositivi con lo stesso nome */
const nomeDisp = d => d.etichetta || d.alias;
/* quattro cose diverse: ricordato, vicino (presenza), raggiungibile (connessione riuscita), confermato (verifica con codice) */
const statoDisp = d => d.vicino ? (d.raggiungibile === false ? "Visto, ma non risponde" : d.raggiungibile ? "Raggiungibile" : "Vicino")
  : (d.ricordato ? "Ricordato · non vicino" : "Non vicino");
function set(el, html) { if (typeof el === "string") el = $(el); if (el && el._h !== html) { el._h = html; el.innerHTML = html; icone(el); } }
/* canale verso il motore (comandi Tauri: niente server HTTP, niente token) */
const TAURI = window.__TAURI__;
async function api(m, a) {
  try { return await TAURI.core.invoke("api", { m, a: a || {} }); } catch (e) { return { ok: false }; }
}
function toast(testo, tipo = "ok") {
  const t = document.createElement("div");
  t.className = "toast vetro";
  t.innerHTML = (tipo === "ok" ? `<span style="color:var(--verde)">${I.ok}</span>` : `<span style="color:var(--arancio)">${I.attenzione}</span>`) + esc(testo);
  $("#toasts").appendChild(t);
  setTimeout(() => { t.style.transition = ".3s"; t.style.opacity = 0; t.style.transform = "translateY(-10px)"; }, 2800);
  setTimeout(() => t.remove(), 3200);
}
/* conferma in pagina (il confirm() del browser non è disponibile in WebKitGTK) */
function chiedi({ titolo, testo, ok = "Conferma", pericolo = false }) {
  return new Promise(res => {
    set("#sheetConf", `<h3>${esc(titolo)}</h3><p style="margin:10px 0 20px;line-height:1.45">${esc(testo)}</p>
      <div class="azioni"><button class="btn" id="confNo">Annulla</button><button class="btn ${pericolo ? "rosso" : "primario"}" id="confSi">${esc(ok)}</button></div>`);
    $("#veloConf").classList.add("on");
    const chiudi = v => { $("#veloConf").classList.remove("on"); res(v); };
    $("#confNo").onclick = () => chiudi(false); $("#confSi").onclick = () => chiudi(true);
  });
}
const MODELLI = {};

/* ------------------------------------------------------------ stato */
let S = null, visto = Date.now() / 1000, pagina = "home", destSel = null, coda = [], sheetId = null, pinKey = null;
const iconaTipo = t => I[{ foto: "foto", video: "video", audio: "audio", pdf: "pdf", testo: "testo", archivio: "archivio", cartella: "cartella" }[t] || "file"];
const coloreTipo = t => ({ foto: "#ff9f0a", video: "#ff375f", audio: "#bf5af2", pdf: "#ff453a", testo: "#64d2ff", archivio: "#a2845e", cartella: "#0a84ff" }[t] || "#8e8e93");
const FINITE = ["completata", "parziale", "rifiutata", "scaduta", "annullata", "errore"];

function vicini() { return (S?.dispositivi || []).filter(d => d.vicino); }
/* il destinatario è quello scelto, oppure l'unico dispositivo vicino; mai una scelta implicita fra più */
function destinazione() {
  const v = vicini();
  if (destSel) { const d = v.find(x => x.fp === destSel); if (d) return d; }
  return v.length === 1 ? v[0] : null;
}
function nessunDest() { return vicini().length > 1 ? "Scegli un dispositivo in Condividi" : "Apri LocalSend sullo smartphone"; }
function tipoDa(n) {
  const e = (n.split(".").pop() || "").toLowerCase();
  if (["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp"].includes(e)) return "foto";
  if (["mov", "mp4", "m4v", "mkv", "webm", "avi"].includes(e)) return "video";
  if (["mp3", "m4a", "wav", "aac", "flac", "ogg"].includes(e)) return "audio";
  if (e === "pdf") return "pdf";
  if (["zip", "rar", "7z", "gz", "tar"].includes(e)) return "archivio";
  return "file";
}
const daPercorso = p => ({ path: p, nome: p.split("/").filter(Boolean).pop() || p, tipo: tipoDa(p) });

/* ------------------------------------------------------------ navigazione */
function vai(p) {
  pagina = p;
  document.querySelectorAll(".pagina").forEach(s => s.classList.toggle("on", s.dataset.p === p));
  document.querySelectorAll(".tab").forEach(t => t.classList.toggle("on", t.dataset.p === p));
  $("#barraTitolo").textContent = document.querySelector(`.pagina[data-p="${p}"]`).dataset.t;
  $("#main").scrollTo({ top: 0, behavior: "instant" });
  try { localStorage.setItem("hub.pagina", p); } catch (e) {}
  if (S) render();
}
document.querySelectorAll(".tab").forEach(t => t.onclick = () => vai(t.dataset.p));
$("#main").addEventListener("scroll", () => $("#barra").classList.toggle("su", $("#main").scrollTop > 40));

/* ------------------------------------------------------------ tema e vetro */
function applicaAspetto() {
  const c = S.conf;
  const scuro = c.tema === "scuro" || (c.tema === "auto" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.tema = scuro ? "scuro" : "chiaro";
  if (!applicaAspetto.trascina) document.documentElement.style.setProperty("--tint", (c.vetro / 100).toFixed(2));
}
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => S && applicaAspetto());

/* ------------------------------------------------------------ render */
function render() {
  if (!S) return;
  applicaAspetto();
  renderHome(); renderSchermo(); renderCondividi(); renderCavo(); renderImpostazioni(); renderSheet(); renderPin(); renderScelta(); renderWizard();
  if (pagina === "condividi") visto = Date.now() / 1000;
  const pend = S.ricezioni.filter(r => r.stato === "attesa").length + S.messaggi.filter(m => m.quando > visto).length;
  const p = $("#puntoCond"); p.textContent = pend; p.classList.toggle("on", pend > 0 && pagina !== "condividi");
}

function saluto() {
  const h = new Date().getHours(), n = S.utente ? ", " + S.utente : "";
  return (h < 5 ? "Buonanotte" : h < 13 ? "Buongiorno" : h < 18 ? "Buon pomeriggio" : "Buonasera") + n;
}

function avvisoLS() {
  const sv = S.server;
  let h = "";
  if (S.identita.stato !== "ok") {
    h += `<div class="vetro avviso"><div class="ic">${I.attenzione}</div><div class="t"><b>Identità non valida</b>
      <small>${esc(S.identita.errore)} Non la rigenero da sola: decidi tu in Impostazioni › Identità.</small></div>
      <button class="btn piccolo primario" data-azione="vai" data-p="impostazioni">Apri</button></div>`;
  } else if (!sv.attivo) {
    const porta = sv.errore === "porta";
    h += `<div class="vetro avviso"><div class="ic">${I.attenzione}</div><div class="t">
      <b>${porta ? "Un'altra app sta già usando la porta " + sv.porta : "Ricezione file non attiva"}</b>
      <small>${porta ? "Di solito è LocalSend per PC: chiudilo, poi riprova. L'invio verso altri dispositivi non è influenzato." : esc(sv.errore || "")}</small></div>
      <button class="btn piccolo primario" data-azione="riprovaLS">Riprova</button></div>`;
  }
  for (const c of S.cambi_identita || []) {
    h += `<div class="vetro avviso"><div class="ic">${I.attenzione}</div><div class="t"><b>«${esc(c.nome)}» si presenta con un'identità diversa</b>
      <small>Ha lo stesso nome di un dispositivo ricordato${c.vecchio_confermato ? " e confermato da te" : ""}, ma il certificato è diverso (${esc(c.nuovo.slice(0, 8))} invece di ${esc(c.antico.slice(0, 8))}). Può essere l'app reinstallata oppure un altro dispositivo. Il vecchio resta com'è; il nuovo non è confermato.</small></div>
      <button class="btn piccolo" data-azione="idIgnora" data-nuovo="${esc(c.nuovo)}">Ignora</button>
      <button class="btn piccolo primario" data-azione="idSostituisci" data-antico="${esc(c.antico)}" data-nuovo="${esc(c.nuovo)}" data-nome="${esc(c.nome)}">È lo stesso: sostituisci</button></div>`;
  }
  for (const a of S.avvisi || []) h += `<div class="vetro avviso"><div class="ic" style="background:var(--blu)">${I.ok}</div><div class="t"><small style="font-size:13.5px">${esc(a)}</small></div></div>`;
  return h;
}

function renderHome() {
  set("#saluto", esc(saluto()));
  set("#data", esc(new Date().toLocaleDateString("it-IT", { weekday: "long", day: "numeric", month: "long" }).replace(/^./, c => c.toUpperCase())));
  set("#avvisoLS", avvisoLS());

  // card dispositivo
  const cv = S.adb, info = cv.info || {}, dest = destinazione(), v = vicini();
  let h;
  if (cv.stato === "collegato") {
    const batt = +info.BatteryCurrentCapacity || 0, carica = info.BatteryIsCharging === "true";
    const tot = +info.TotalDiskCapacity || 0, lib = +info.AmountDataAvailable || 0, usato = tot ? Math.round((tot - lib) / tot * 100) : 0;
    const col = batt <= 20 ? "var(--rosso)" : "var(--verde)";
    h = `<h2>Il tuo telefono</h2><div class="pixel"><div class="telefono"></div><div class="info">
      <h3>${esc(info.DeviceName || "Android")}</h3><div class="sub">${esc(MODELLI[info.ProductType] || info.ProductType || "Android")} · Android ${esc(info.ProductVersion)}</div>
      <span class="chip ok"><i></i>Collegato tramite ADB</span>${dest ? ' <span class="chip blu"><i></i>Anche in Wi-Fi</span>' : ""}
      <div class="misure"><div class="anello"><svg viewBox="0 0 44 44"><circle cx="22" cy="22" r="18" stroke="var(--riempi)" stroke-width="5" fill="none"/>
        <circle cx="22" cy="22" r="18" stroke="${col}" stroke-width="5" fill="none" stroke-linecap="round" stroke-dasharray="${(batt / 100 * 113).toFixed(1)} 113"/></svg>
        <div><b>${batt ? batt + "%" : "—"}</b><small>${carica ? "In carica" : "Batteria"}</small></div></div>
        ${tot ? `<div class="spazio"><b>${dim(lib)} liberi</b><div class="barra-prog"><i style="width:${usato}%;background:${usato > 90 ? "var(--rosso)" : "var(--blu)"}"></i></div><small>usati ${dim(tot - lib)} di ${dim(tot)}</small></div>` : ""}
      </div></div></div>`;
  } else if (dest) {
    h = `<h2>Destinatario</h2><div class="pixel"><div class="telefono"></div><div class="info">
      <h3>${esc(nomeDisp(dest))}</h3><div class="sub">${esc(dest.modello || "Dispositivo")} · ${esc(dest.ip)}</div>
      <span class="chip ${dest.raggiungibile === false ? "warn" : "ok"}"><i></i>${esc(statoDisp(dest))}</span>${dest.confermato ? ' <span class="chip blu">Identità confermata da te</span>' : ""}${dest.preferito ? ' <span class="chip">★ Preferito</span>' : ""}
      <p class="vuoto" style="margin-top:14px">${v.length > 1 ? `Ci sono altri ${v.length - 1} dispositivi vicini: li scegli in <span class="link" data-azione="vai" data-p="condividi">Condividi</span>.` : "Trascina file qui sotto per mandarglieli."}</p></div></div>`;
  } else {
    h = `<h2>Dispositivi</h2><div class="pixel"><div class="telefono spento"></div><div class="info">
      <h3>${v.length ? "Scegli un dispositivo" : "Non vedo nessuno"}</h3><div class="sub">${v.length ? v.length + " vicini: scegli il destinatario in Condividi" : "Per collegare un telefono:"}</div>
      ${v.length ? `<button class="btn primario" data-azione="vai" data-p="condividi">Vai a Condividi</button>` :
      `<div class="passi" style="gap:10px"><div class="passo"><div class="n" style="background:var(--verde)">${I.wifi.replace("<svg", '<svg width="16" height="16"')}</div><div><b>Apri LocalSend sul telefono</b><small>Stessa Wi-Fi o hotspot del telefono</small></div></div>
      <div class="passo"><div class="n" style="background:var(--arancio)">${I.cavo.replace("<svg", '<svg width="16" height="16"')}</div><div><b>Facoltativo: collega il cavo</b><small>Solo per schermo, batteria e importazione foto</small></div></div></div>`}
      </div></div>`;
  }
  set("#cardPixel", h);

  // schermo
  const sc = S.schermo;
  let m;
  if (!sc.installato) m = `<div class="tasto-tondo" style="background:var(--riempi);color:var(--txt3)">${I.mirror}</div><b>scrcpy non installato</b><small>Funzione facoltativa: vedi Impostazioni › Componenti.</small>`;
  else if (sc.connesso) m = `<button class="tasto-tondo attivo" data-azione="schermo">${I.stop}</button><b>Schermo collegato</b><small>Lo schermo del telefono è nella sua finestra. Tocca per fermare.</small>`;
  else if (sc.attivo) m = `<button class="tasto-tondo attivo" data-azione="schermo">${I.stop}</button><b>Avvio scrcpy…</b><small>Controlla il telefono e i dettagli in Schermo.</small>`;
  else m = `<button class="tasto-tondo" data-azione="schermo">${I.play}</button><b>Schermo del telefono</b><small>Collega il telefono con Debug USB e premi Avvia.</small>`;
  set("#homeMirror", m);
  $("#cardMirror").style.display = sc.installato ? "" : "none";          // facoltativo, solo Android

  set("#homeDest", dest ? `<span class="chip blu"><i></i>a ${esc(nomeDisp(dest))}</span>` : `<span class="chip warn"><i class="pulsa"></i>${esc(nessunDest())}</span>`);
  set("#homeInvii", listaInvii(S.invii.slice(0, 2), true));

  const msg = S.messaggi[0];
  set("#homeMsg", msg ? bolla(msg) : "");

  const rec = S.recenti.slice(0, 12);
  set("#homeRecenti", rec.length ? rec.map(tile).join("") : `<p class="vuoto">Quando un dispositivo ti manda qualcosa, lo trovi qui. Sullo smartphone: Condividi › LocalSend (o app compatibile) › «${esc(S.conf.nome_pc)}».</p>`);
}

function bolla(m) {
  return `<div class="bolla">${esc(m.testo)}</div><div class="bolla-meta">${I.messaggio.replace("<svg", '<svg width="14" height="14"')} ${esc(m.alias)} · ${fa(m.quando)}
    <span class="dx"><button class="btn piccolo" data-azione="copia" data-id="${esc(m.id)}">${I.copia}Copia</button>
    ${/^https?:\/\/\S+$/.test(m.testo.trim()) ? `<button class="btn piccolo" data-azione="apriLink" data-id="${esc(m.id)}">${I.mondo}Apri</button>` : ""}
    <button class="btn piccolo icona" data-azione="togliMsg" data-id="${esc(m.id)}" title="Elimina">${I.x}</button></span></div>`;
}

function tile(f) {
  const img = f.tipo === "foto" && !/\.hei[cf]$/i.test(f.nome);
  const sty = img ? `style="background-image:url('${TAURI.core.convertFileSrc(f.path)}')"` : `style="color:${coloreTipo(f.tipo)}"`;
  return `<div class="tile" draggable="true" data-file="${esc(f.path)}" title="${esc(f.nome)}"><div class="img" ${sty}>${img ? "" : iconaTipo(f.tipo)}</div>
    <b>${esc(f.nome)}</b><small>${f.dir ? "Cartella" : dim(f.size)} · ${fa(f.mtime)}</small></div>`;
}

const STATO_INVIO = {
  preparazione: () => "Preparazione: controllo i file e cerco il dispositivo…", attesa: j => `In attesa che ${j.verso} accetti…`,
  pin: j => j.pin_errato ? "PIN errato: riprova" : "Serve il PIN del destinatario",
  invio: (j, pct) => `Trasferimento ${pct}% · ${dim(j.inviati)} di ${dim(j.totale)}${vel(j.velocita)}${j.residuo ? " · circa " + dur(j.residuo) + " rimanenti" : ""}`,
  completato: () => "Completato ✓",
  parziale: j => j.errore || "Completato solo in parte", rifiutato: j => j.errore || "Rifiutato dal destinatario",
  annullato: j => "Annullato da te" + (j.errore ? " · " + j.errore : ""),
  scaduto: j => "Scaduto · " + (j.errore || "nessuna risposta"), interrotto: j => "Interrotto · " + (j.errore || "connessione persa"), errore: j => j.errore || "Errore",
};
const STATO_FILE = { attesa: "in coda", inviando: "in corso", fatto: "inviato ✓", errore: "non inviato", rifiutato: "non accettato dal destinatario" };
const dettAperti = new Set();
document.addEventListener("toggle", e => { const id = e.target?.dataset?.jid; if (id) { e.target.open ? dettAperti.add(id) : dettAperti.delete(id); } }, true);
function dettaglioFile(j) {
  if (!(j.file.length > 1 || j.errori)) return "";
  const righe = j.file.map(f => {
    const pct = f.size ? Math.round(f.inviati / f.size * 100) : 100;
    const t = f.stato === "inviando" ? `${pct}% · ${dim(f.inviati)} di ${dim(f.size)}` : (STATO_FILE[f.stato] || f.stato) + (f.incerto ? " · esito incerto" : "");
    return `<div class="f ${f.stato === "errore" ? "bad" : ""}"><b title="${esc(f.nome)}">${esc(f.nome)}</b><small title="${esc(f.errore)}">${esc(t)}</small></div>`;
  }).join("") + (j.altri_file ? `<div class="f"><small>…e altri ${j.altri_file} file</small></div>` : "");
  return `<details class="dett" data-jid="${esc(j.id)}" ${dettAperti.has(j.id) ? "open" : ""}><summary>Esito per file · ${j.fatti} inviati${j.errori ? `, ${j.errori} non inviati` : ""}${j.rifiutati ? `, ${j.rifiutati} non accettati` : ""}</summary>${righe}</details>`;
}
function listaInvii(lista, compatta) {
  if (!lista.length) return compatta ? "" : `<p class="vuoto">Nessun trasferimento in corso.</p>`;
  return `<div style="margin-top:${compatta ? 14 : 0}px">` + lista.map(j => {
    const pct = j.totale ? Math.round(j.inviati / j.totale * 100) : 0;
    const st = (STATO_INVIO[j.stato] || (() => j.stato))(j, pct);
    const bad = ["errore", "rifiutato"].includes(j.stato), warn = ["parziale", "pin", "scaduto", "interrotto"].includes(j.stato);
    const col = bad ? "var(--rosso)" : warn ? "var(--arancio)" : j.stato === "completato" ? "var(--verde)" : "var(--blu)";
    const nome = j.testo ? "Testo" : j.n === 1 ? j.nomi[0] : `${j.n} file`;
    const viva = ["preparazione", "attesa", "pin", "invio"].includes(j.stato);
    const attuale = j.stato === "invio" ? j.file.find(f => f.stato === "inviando") : null;
    const riprova = j.riprovabili > 0 && !j.riprovato ? `<div style="margin-top:8px"><button class="btn piccolo primario" data-azione="riprova" data-id="${esc(j.id)}">Riprova i file non completati (${j.riprovabili})</button>
      <small class="vuoto" style="display:block;margin-top:4px">Ogni file riparte dall'inizio; quelli già inviati non vengono rimandati.</small></div>` : "";
    return `<div class="trasf"><div class="ic" style="background:color-mix(in srgb,${col} 15%,transparent);color:${col}">${j.stato === "completato" ? I.ok : bad || warn ? I.attenzione : I.su}</div>
      <div class="t"><b>${esc(nome)} → ${esc(j.verso)}</b><small style="${bad ? "color:var(--rosso)" : ""}">${esc(st)}</small>
      ${j.stato === "invio" || j.stato === "attesa" ? `<div class="barra-prog"><i style="width:${j.stato === "attesa" ? 3 : pct}%"></i></div>` : ""}
      ${attuale && j.n > 1 ? `<small class="vuoto">${esc(attuale.nome)} · ${attuale.size ? Math.round(attuale.inviati / attuale.size * 100) : 0}% · file ${j.fatti + 1} di ${j.n - j.rifiutati}</small>` : ""}
      ${compatta ? "" : dettaglioFile(j)}${riprova}</div>
      ${viva ? `<button class="btn icona piccolo" data-azione="annullaInvio" data-id="${esc(j.id)}" title="Annulla">${I.x}</button>` : ""}</div>`;
  }).join("") + "</div>";
}

/* «Apri» (un solo file: il file; altrimenti la cartella) e «Mostra nella cartella» */
function aperturaRicevuti(r) {
  const fatti = r.file.filter(f => f.stato === "fatto" && f.percorso);
  const primo = fatti[0]?.percorso || "";
  return `<span style="display:flex;gap:6px;flex-direction:column;align-items:stretch">${fatti.length === 1 ? `<button class="btn piccolo" data-azione="apriFile" data-path="${esc(primo)}">Apri</button>` : `<button class="btn piccolo" data-azione="apriCartella">Apri cartella</button>`}
    ${primo ? `<button class="btn piccolo" data-azione="mostra" data-path="${esc(primo)}">Mostra nella cartella</button>` : ""}</span>`;
}
const STATO_RIC = { completata: "Ricevuto ✓", parziale: "Ricevuto solo in parte", rifiutata: "Rifiutato da te", scaduta: "Scaduto", annullata: "Annullato", errore: "Non riuscito" };
function listaRicezioni() {
  const attive = S.ricezioni.filter(r => r.stato === "ricezione" || r.stato === "accettata").map(r => {
    const pct = r.totale ? Math.round(r.byte / r.totale * 100) : 0;
    return `<div class="trasf"><div class="ic" style="background:color-mix(in srgb,var(--verde) 15%,transparent);color:var(--verde)">${I.su.replace("<svg", '<svg style="transform:rotate(180deg)"')}</div>
      <div class="t"><b>Da ${esc(r.alias)} · ${r.selezionati} file</b><small>Ricevo… ${pct}% · ${dim(r.byte)} di ${dim(r.totale)}${vel(r.velocita)}</small><div class="barra-prog"><i style="width:${pct}%;background:var(--verde)"></i></div></div>
      <button class="btn icona piccolo" data-azione="annullaRicezione" data-id="${esc(r.id)}" title="Annulla">${I.x}</button></div>`;
  }).join("");
  const fatte = S.ricezioni.filter(r => FINITE.includes(r.stato) && r.tipo === "file").slice(0, 5).map(r => {
    const ok = r.stato === "completata", bad = r.stato === "errore";
    const col = ok ? "var(--verde)" : bad ? "var(--rosso)" : "var(--arancio)";
    return `<div class="trasf"><div class="ic" style="background:color-mix(in srgb,${col} 15%,transparent);color:${col}">${ok ? I.ok : I.attenzione}</div>
      <div class="t"><b>Da ${esc(r.alias)}</b><small>${esc(STATO_RIC[r.stato])}${r.ricevuti ? ` · ${r.ricevuti} di ${r.selezionati} file` : ""}${r.errore && !ok ? " · " + esc(r.errore) : ""}</small></div>
      ${r.ricevuti ? aperturaRicevuti(r) : ""}</div>`;
  }).join("");
  return attive + fatte;
}

function renderSchermo() {
  const sc = S.schermo;
  set("#passo3", "Premi Avvia in questa pagina");
  $("#onde").classList.toggle("on", sc.attivo && !sc.connesso);
  $("#telSchermo").classList.toggle("spento", !sc.connesso);
  let h;
  if (!sc.installato) h = `<h3>scrcpy non installato</h3><p class="vuoto">Funzione facoltativa, indipendente dai trasferimenti Wi-Fi. Per attivarla vedi Impostazioni › Componenti.</p>`;
  else if (sc.attivo) h = `<h3>${sc.connesso ? "Schermo collegato" : "Avvio scrcpy…"}</h3><p class="vuoto">Schermo e controllo del telefono in una finestra separata. Se non compare, leggi i dettagli tecnici qui sotto.</p><button class="btn rosso grande" data-azione="schermo">${I.stop}Ferma</button>`;
  else h = `<h3>Il telefono sul PC</h3><p class="vuoto">Collega un solo telefono, abilita Debug USB e autorizza Fedora. Poi premi Avvia.</p><button class="btn primario grande" data-azione="schermo">${I.play}Avvia</button>`;
  set("#schermoStato", h);
  const pre = $("#schermoLog"), txt = sc.log.join("\n") || "Nessun messaggio.";
  if (pre.textContent !== txt) { pre.textContent = txt; pre.scrollTop = pre.scrollHeight; }
}

function renderCondividi() {
  set("#avvisoLS2", avvisoLS());
  const dest = destinazione(), tutti = S.dispositivi;
  let h = tutti.map(d => `<div class="disp ${dest && d.fp === dest.fp ? "sel" : ""} ${d.vicino ? "" : "offline"}" data-fp="${esc(d.fp)}" title="${esc((d.ip || "") + (d.modello ? " · " + d.modello : "") + " · impronta " + d.fp.slice(0, 8))}">
      <div class="av" style="${d.tipo === "desktop" ? "background:linear-gradient(160deg,#9aa0a6,#5f6368)" : ""}">${d.tipo === "desktop" ? I.pc : esc(iniziale(d.alias))}
        <button class="stella ${d.preferito ? "on" : ""}" data-azione="stella" data-fp="${esc(d.fp)}" data-on="${d.preferito ? 0 : 1}" title="${d.preferito ? "Togli dai preferiti" : "Aggiungi ai preferiti"}">${d.preferito ? I.stellaPiena : I.stella}</button></div>
      <b>${esc(nomeDisp(d))}</b><small>${esc(statoDisp(d))}${d.confermato ? " · ✓" : ""}</small></div>`).join("");
  h += `<div class="disp" style="cursor:default"><div class="radar">${I.telefono}</div><b>${vicini().length ? "Cerco altri…" : "Cerco…"}</b><small>${vicini().length ? "" : "Apri LocalSend sullo smartphone"}</small></div>`;
  set("#vicini", h);

  set("#coda", coda.map((f, i) => `<span class="chip">${iconaTipo(f.tipo).replace("<svg", '<svg width="14" height="14"')}${esc(f.nome)}<button data-azione="togliCoda" data-i="${i}" title="Togli">${I.x}</button></span>`).join(""));
  const b = $("#inviaCoda");
  b.disabled = !coda.length || !dest;
  $("#inviaCodaT").textContent = !coda.length ? "Invia" : dest ? `Invia a ${nomeDisp(dest)}` : (vicini().length > 1 ? "Scegli un dispositivo" : "Nessun dispositivo vicino");

  const t = listaRicezioni() + (S.invii.length ? listaInvii(S.invii.slice(0, 6), false) : "");
  set("#trasferimenti", t || `<p class="vuoto">Nessun trasferimento in corso.</p>`);

  set("#messaggi", S.messaggi.length ? S.messaggi.slice(0, 6).map(bolla).join("")
    : `<p class="vuoto">I testi che mandi dallo smartphone (in LocalSend: Invia › Testo) compaiono qui, pronti da copiare.</p>`);

  set("#ricevuti", S.recenti.length ? S.recenti.map(f => `<div class="riga cliccabile" draggable="true" data-file="${esc(f.path)}">
      <div class="ic" style="background:${coloreTipo(f.tipo)}">${iconaTipo(f.tipo)}</div>
      <div class="testo"><b>${esc(f.nome)}</b><small>${f.dir ? "Cartella" : dim(f.size)} · ${fa(f.mtime)}</small></div>
      <button class="btn icona piccolo" data-azione="mostra" data-path="${esc(f.path)}" title="Mostra nella cartella">${I.cartella}</button></div>`).join("")
    : `<div class="riga"><div class="testo"><b style="color:var(--txt2)">Ancora niente</b><small>Cartella: ${esc(S.conf.cartella)}</small></div></div>`);
}

function renderCavo() {
  const cv = S.adb, info = cv.info || {}, imp = cv.importa;
  let h;
  if (cv.stato === "mancante") h = `<div class="vetro card" style="text-align:center;padding:50px 22px"><div class="telefono spento" style="margin:0 auto 20px"></div><h3 style="font-size:22px">ADB non è installato</h3><p class="vuoto" style="margin:6px 0 18px">Funzione facoltativa: i trasferimenti Wi-Fi funzionano comunque. Vedi Impostazioni › Componenti.</p><button class="btn primario" data-azione="vai" data-p="impostazioni">Vai a Impostazioni</button></div>`;
  else if (cv.stato === "errore") h = `<div class="vetro card"><h3>Controlla il collegamento</h3><p>${esc(cv.errore)}</p></div>`;
  else if (cv.stato === "assente") h = `<div class="vetro card" style="text-align:center;padding:50px 22px"><div class="telefono spento" style="margin:0 auto 20px"></div><h3 style="font-size:22px">Collega il telefono con il cavo</h3><p class="vuoto" style="margin:6px auto 0;max-width:380px">Abilita Debug USB nelle Opzioni sviluppatore, collega un cavo dati e accetta «Consenti debug USB» sul telefono.</p></div>`;
  else if (cv.stato === "fiducia") h = `<div class="vetro card" style="text-align:center;padding:50px 22px"><div class="tasto-tondo" style="margin:0 auto 20px;background:linear-gradient(160deg,#ffb340,var(--arancio))">${I.lucchetto}</div><h3 style="font-size:22px">Autorizza questo computer</h3><p class="vuoto" style="margin:6px auto 18px;max-width:380px">Sul telefono sblocca lo schermo e accetta «Consenti debug USB». Poi premi Riprova.</p><button class="btn primario" data-azione="associa">Riprova</button></div>`;
  else {
    const batt = +info.BatteryCurrentCapacity || 0, carica = info.BatteryIsCharging === "true";
    const tot = +info.TotalDiskCapacity || 0, lib = +info.AmountDataAvailable || 0, usato = tot ? Math.round((tot - lib) / tot * 100) : 0;
    const pct = imp.tot ? Math.round(imp.fatti / imp.tot * 100) : 0;
    h = `<div class="griglia">
      <div class="vetro card c12"><div class="pixel"><div class="telefono"></div><div class="info">
        <h3 style="font-size:28px">${esc(info.DeviceName || "Android")}</h3><div class="sub">${esc(MODELLI[info.ProductType] || info.ProductType || "Android")} · Android ${esc(info.ProductVersion)}</div>
        <span class="chip ok"><i></i>Collegato tramite ADB</span></div></div></div>
      <div class="vetro card c6"><h2>Batteria</h2><div class="anello"><svg viewBox="0 0 44 44" style="width:64px;height:64px"><circle cx="22" cy="22" r="18" stroke="var(--riempi)" stroke-width="5" fill="none"/>
        <circle cx="22" cy="22" r="18" stroke="${batt <= 20 ? "var(--rosso)" : "var(--verde)"}" stroke-width="5" fill="none" stroke-linecap="round" stroke-dasharray="${(batt / 100 * 113).toFixed(1)} 113"/></svg>
        <div><b style="font-size:26px">${batt ? batt + "%" : "—"}</b><small>${carica ? "In carica" : "Non in carica"}</small></div></div></div>
      <div class="vetro card c6"><h2>Spazio</h2>${tot ? `<div class="spazio"><b style="font-size:26px">${dim(lib)}</b> <span class="vuoto">liberi</span><div class="barra-prog" style="height:10px"><i style="width:${usato}%;background:${usato > 90 ? "var(--rosso)" : "var(--blu)"}"></i></div><small>usati ${dim(tot - lib)} di ${dim(tot)}</small></div>` : `<p class="vuoto">Non disponibile</p>`}</div>
      <div class="vetro card c8"><h2>Foto e video</h2>
        <p class="vuoto" style="margin-bottom:14px">Importo DCIM locale (salto i file con stesso percorso e dimensione) in <b>${esc(cv.destinazione)}</b>. Sul telefono non cancello niente.</p>
        ${imp.attivo ? `<div class="barra-prog" style="height:10px"><i style="width:${imp.tot ? pct : 4}%"></i></div><p class="vuoto" style="margin-top:8px">${esc(imp.msg)} ${imp.tot ? `${imp.fatti} di ${imp.tot}` : ""}</p>`
        : `<div style="display:flex;gap:10px;flex-wrap:wrap"><button class="btn primario" data-azione="importa">${I.su.replace("<svg", '<svg style="transform:rotate(180deg)"')}Importa foto e video</button><button class="btn" data-azione="apriCartella" data-q="foto">${I.cartella}Apri la cartella</button></div>
           ${imp.msg ? `<p class="vuoto" style="margin-top:10px">${esc(imp.msg)}</p>` : ""}`}</div>
      <div class="vetro card c4"><h2>File</h2><p class="vuoto" style="margin-bottom:14px">Sul telefono seleziona USB › Trasferimento file. Poi apri il telefono in File. Le cartelle private delle app non sono accessibili.</p><button class="btn" data-azione="cavoApri">${I.cartella}Apri in File</button></div>
    </div>`;
  }
  set("#cavo", h);
}

function renderImpostazioni() {
  const c = S.conf;
  document.querySelectorAll("[data-conf]").forEach(i => { if (document.activeElement !== i) i.checked = !!c[i.dataset.conf]; });
  document.querySelectorAll("[data-conf-testo]").forEach(i => { if (document.activeElement !== i) i.value = c[i.dataset.confTesto]; });
  document.querySelectorAll("#segTema button").forEach(b => b.classList.toggle("on", b.dataset.v === c.tema));
  if (!applicaAspetto.trascina) $("#vetro").value = c.vetro;
  set("#impCartella", esc(c.cartella));
  set("#impFoto", esc(c.cartella_foto));
  set("#pinStato", S.pin_impostato ? "Impostato: i mittenti devono inserirlo" : "Non impostato: ogni richiesta resta comunque da confermare");
  $("#pinTogli").style.display = S.pin_impostato ? "" : "none";
  const ric = S.dispositivi.filter(d => d.ricordato);
  set("#preferiti", ric.length ? `<div class="sezione">Dispositivi ricordati</div><div class="vetro lista">` + ric.map(f =>
    `<div class="riga"><div class="ic" style="background:linear-gradient(160deg,#7aa2ff,#5e5ce6)">${I.telefono}</div><div class="testo"><b>${esc(nomeDisp(f))}</b>
     <small>${esc(statoDisp(f))} · ${f.confermato ? "identità confermata da te" : "identità non confermata"} · impronta ${esc(f.fp.slice(0, 8))}</small></div>
     <button class="btn piccolo icona" data-azione="stella" data-fp="${esc(f.fp)}" data-on="${f.preferito ? 0 : 1}" title="${f.preferito ? "Togli la stella" : "Metti la stella"}">${f.preferito ? I.stellaPiena : I.stella}</button>
     <button class="btn piccolo" data-azione="dimentica" data-fp="${esc(f.fp)}" data-nome="${esc(f.alias)}">Dimentica</button></div>`).join("") +
    `</div><p class="vuoto" style="margin:8px 8px 0">La stella è solo una scorciatoia (il dispositivo resta in cima all'elenco): non autorizza mai la ricezione automatica e il nome non prova chi sia il dispositivo. «Confermato» significa che hai verificato l'impronta con il codice.</p>` : "");
  set("#componenti", S.componenti.map(k => `<div class="riga"><div class="ic" style="background:${k.ok ? "var(--verde)" : "var(--arancio)"}">${k.ok ? I.ok : I.attenzione}</div>
    <div class="testo"><b>${esc(k.nome)}</b><small>${esc(k.desc)}${k.ok ? "" : " · nel terminale: " + esc(k.comando)}</small></div><span class="val">${k.ok ? "Pronto" : "Non installato"}</span></div>`).join(""));
  const r = S.rete;
  set("#rete", `<div class="riga"><div class="ic" style="background:var(--blu)">${I.wifi}</div><div class="testo"><b>Connessione</b></div><span class="val">${esc(r.nome || "—")}</span></div>
    <div class="riga"><div class="ic" style="background:#8e8e93">${I.rete}</div><div class="testo"><b>Indirizzo del PC</b></div><span class="val" style="user-select:text">${esc(r.ip.join(", ") || "—")}</span></div>
    <div class="riga"><div class="ic" style="background:${S.server.attivo ? "var(--verde)" : "var(--rosso)"}">${I.lucchetto}</div><div class="testo"><b>Ricezione LocalSend</b><small>Connessione cifrata (HTTPS) · porta ${S.server.porta}</small></div><span class="val">${S.server.attivo ? "Attiva" : "Spenta"}</span></div>`);
  const id = S.identita;
  set("#identita", `<div class="riga"><div class="ic" style="background:${id.stato === "ok" ? "var(--verde)" : "var(--rosso)"}">${I.scudo}</div><div class="testo"><b>Identità di questo PC</b>
    <small style="user-select:text">${id.stato === "ok" ? esc(id.impronta.match(/.{1,8}/g).slice(0, 4).join(" ")) + "…" : esc(id.errore)}</small></div><span class="val">${id.stato === "ok" ? "Valida" : "Da correggere"}</span></div>`);
  set("#versione", `Cable ${esc(S.versione)} · protocollo LocalSend · ADB e scrcpy facoltativi (solo Android)`);
  const andr = S.componenti.some(k => k.ok);                              // strumenti Android: visibili solo se installati
  $("#tabSchermo").style.display = $("#tabPixel").style.display = andr ? "" : "none";
}

/* ------------------------------------------------------------ richieste in arrivo (accettazione anche parziale) */
function renderSheet() {
  const r = S.ricezioni.find(x => x.stato === "attesa");
  const velo = $("#velo");
  if (!r) { if (sheetId) { velo.classList.remove("on"); sheetId = null; } return; }
  if (sheetId === r.id) return;
  sheetId = r.id;
  const n = r.file.length, testo = r.tipo === "testo";
  set("#sheet", `<div class="av">${esc(iniziale(r.alias))}</div>
    <h3>${esc(r.alias)}</h3><p>${testo ? "vuole inviarti un messaggio di testo" : `vuole inviarti ${n === 1 ? "un file" : n + " file"} · ${dim(r.totale)}`}</p>
    <p style="font-size:12.5px;margin-top:6px">Indirizzo ${esc(r.ip)} · ${r.noto ? `corrisponde a «${esc(r.noto.nome)}» (${r.noto.confermato ? "confermato da te" : r.noto.ricordato ? "dispositivo ricordato" : "visto di recente"}); ` : "dispositivo mai visto prima; "}il nome è dichiarato dal mittente e non è verificato. Accetta solo se lo aspetti.</p>
    ${testo ? "" : `<div class="files">${r.file.slice(0, 200).map(f => `<label><input type="checkbox" data-fid="${esc(f.id)}" checked>${iconaTipo(f.tipo)}<span>${esc(f.nome)}</span><em>${dim(f.size)}</em></label>`).join("")}${r.altri ? `<div><span>…e altri ${r.altri}</span></div>` : ""}</div>`}
    <div class="azioni"><button class="btn" data-azione="rifiuta">Rifiuta</button><button class="btn primario" data-azione="accetta">Accetta</button></div>`);
  velo.classList.add("on");
}

/* ------------------------------------------------------------ PIN richiesto dal destinatario */
function renderPin() {
  const j = S.invii.find(x => x.stato === "pin");
  const velo = $("#veloPin");
  if (!j) { if (pinKey) { velo.classList.remove("on"); pinKey = null; } return; }
  const key = j.id + (j.pin_errato ? "!" : "");
  if (pinKey === key) return;
  pinKey = key;
  set("#sheetPin", `<div class="av" style="background:linear-gradient(160deg,#ffb340,var(--arancio))">${I.lucchetto}</div>
    <h3>${esc(j.verso)} chiede un PIN</h3><p>${j.pin_errato ? "PIN errato. Riprova." : "Inserisci il PIN impostato sul destinatario."}</p>
    <input class="input" id="pinCampo" type="password" autocomplete="off" maxlength="64" style="width:100%;text-align:center;margin:16px 0;height:44px;font-size:20px;letter-spacing:.2em">
    <div class="azioni"><button class="btn" data-azione="pinNo" data-id="${esc(j.id)}">Annulla invio</button><button class="btn primario" data-azione="pinSi" data-id="${esc(j.id)}">Invia</button></div>`);
  velo.classList.add("on");
  setTimeout(() => $("#pinCampo")?.focus(), 50);
}

/* ------------------------------------------------------------ azioni */
async function inviaA(d, percorsi, testo) {
  const r = await api("invia", { fp: d.fp, percorsi, testo });
  toast(r.ok ? `Preparo l'invio a ${nomeDisp(d)}: il dispositivo deve accettare` : (r.errore || "Invio non riuscito"), r.ok ? "ok" : "warn");
  return r.ok;
}
async function scegliFile(subito) {
  const r = await api("scegli_file");
  if (!r.ok || !r.percorsi?.length) return;
  accogli(r.percorsi, subito);
}
/* percorsi reali: subito al destinatario, oppure in coda */
function accogli(percorsi, subito, dest = destinazione()) {
  if (subito && dest) { inviaA(dest, percorsi); return; }
  if (subito) toast(nessunDest() + ": li metto in coda", "warn");
  coda.push(...percorsi.map(daPercorso));
  if (pagina !== "condividi") vai("condividi"); else renderCondividi();
}
/* file portati dentro Cable senza un destinatario (riga di comando, file manager, rilascio sulla finestra):
   si sceglie il destinatario qui, senza passare dalla Home */
let sceltaPercorsi = null;
function chiediDestinatario(percorsi) { sceltaPercorsi = [...new Set((sceltaPercorsi || []).concat(percorsi))]; if (S) renderScelta(); }
function renderScelta() {
  const velo = $("#veloScelta");
  if (!sceltaPercorsi) { velo.classList.remove("on"); return; }
  const v = vicini(), p = sceltaPercorsi.map(daPercorso), n = p.length;
  const quali = n === 1 ? p[0].nome : `${p[0].nome} e altri ${n - 1}`;
  set("#sheetScelta", `<h3>A chi vuoi inviare?</h3><p>${esc(quali)}${n > 1 ? ` · ${n} elementi` : ""}</p>
    <div class="sc-lista">${v.length ? v.map(d => `<div class="disp" data-azione="scegliDest" data-fp="${esc(d.fp)}" title="${esc(d.ip)}">
        <div class="av">${d.tipo === "desktop" ? I.pc : esc(iniziale(d.alias))}</div><b>${esc(nomeDisp(d))}</b><small>${esc(statoDisp(d))}</small></div>`).join("")
      : `<p class="vuoto">Nessun dispositivo vicino. Apri LocalSend sullo smartphone e tocca <span class="link" data-azione="aggiorna">Aggiorna</span>.</p>`}</div>
    <div class="azioni"><button class="btn" data-azione="sceltaChiudi">Annulla</button><button class="btn" data-azione="sceltaCoda">Metti in coda</button></div>`);
  velo.classList.add("on");
}
const AZ = {
  async schermo() { const on = S.schermo.attivo; const r = await api("schermo", { azione: on ? "ferma" : "avvia" }); if (!on && !r.ok) toast("Non riesco ad avviare scrcpy", "warn"); },
  vai(e) { vai(e.dataset.p); },
  async riprovaLS() { const r = await api("riprova_server"); toast(r.ok ? "Ricezione attiva" : "Ancora occupata: chiudi LocalSend per PC", r.ok ? "ok" : "warn"); },
  async accetta() {
    const box = [...document.querySelectorAll("#sheet input[data-fid]")];
    const ids = box.filter(b => b.checked).map(b => b.dataset.fid);
    if (box.length && !ids.length) { toast("Seleziona almeno un file, oppure rifiuta", "warn"); return; }
    await api("decidi", { id: sheetId, ids: ids.length === box.length ? null : ids });
    $("#velo").classList.remove("on"); toast("Ricevo…");
  },
  async rifiuta() { await api("decidi", { id: sheetId, ids: [] }); $("#velo").classList.remove("on"); },
  async copia(e) { const m = S.messaggi.find(x => x.id === e.dataset.id); if (m) { await copiaTesto(m.testo); toast("Copiato negli appunti"); } },
  apriLink(e) { const m = S.messaggi.find(x => x.id === e.dataset.id); if (m) location.href = m.testo.trim(); },
  async togliMsg(e) { await api("msg_rimuovi", { id: e.dataset.id }); },
  async annullaInvio(e) { await api("annulla_invio", { id: e.dataset.id }); },
  async annullaRicezione(e) { await api("annulla_ricezione", { id: e.dataset.id }); },
  togliCoda(e) { coda.splice(+e.dataset.i, 1); renderCondividi(); },
  async mostra(e, ev) { ev.stopPropagation(); await api("mostra", { path: e.dataset.path }); },
  async apriCartella(e) { await api("apri_cartella", { quale: e.dataset.q === "foto" ? "foto" : "file" }); },
  async scegliCartella(e) { const r = await api("scegli_cartella", { quale: e.dataset.q }); if (r.ok) toast("Cartella aggiornata"); },
  async stella(e, ev) { ev.stopPropagation(); await api("preferito", { fp: e.dataset.fp, on: e.dataset.on === "1" }); },
  async dimentica(e) {
    if (await chiedi({ titolo: `Dimenticare «${e.dataset.nome}»?`, ok: "Dimentica", pericolo: true,
      testo: "Viene tolto dai preferiti e dall'elenco. Non viene cancellato nessun file e il dispositivo potrà ricomparire se resta in rete." }))
      { await api("dimentica", { fp: e.dataset.fp }); toast("Dispositivo dimenticato"); }
  },
  async pinSi(e) { const v = $("#pinCampo").value; if (!v) return; await api("pin_invio", { id: e.dataset.id, pin: v }); $("#pinCampo").value = ""; $("#veloPin").classList.remove("on"); pinKey = null; },
  async pinNo(e) { await api("annulla_invio", { id: e.dataset.id }); $("#veloPin").classList.remove("on"); },
  async importa() { const r = await api("android_importa"); if (!r.ok) toast("Non riesco: sblocca il telefono e riprova", "warn"); },
  async cavoApri() { await api("android_apri"); },
  async associa() { const r = await api("android_riprova"); toast(r.ok ? "Autorizzato" : "Sblocca il telefono e accetta «Consenti debug USB»", r.ok ? "ok" : "warn"); },
  async esci() { await api("esci"); },
  async riprova(e) {
    const r = await api("riprova", { id: e.dataset.id });
    toast(r.ok ? (r.saltati?.length ? `Riprovo; ${r.saltati.length} file non più presenti sul PC sono stati saltati` : "Riprovo i file non completati") : (r.errore || "Non posso riprovare"), r.ok ? "ok" : "warn");
  },
  async apriFile(e, ev) { ev.stopPropagation(); await api("apri", { path: e.dataset.path }); },
  async aggiorna() { toast("Aggiorno l'elenco…"); const r = await api("aggiorna"); toast(r.risposto ? `${r.risposto} dispositivi raggiungibili` : "Nessun dispositivo ha risposto: controlla rete e app sullo smartphone", r.risposto ? "ok" : "warn"); },
  async scegliDest(e) {
    const d = (S.dispositivi || []).find(x => x.fp === e.dataset.fp), p = sceltaPercorsi;
    sceltaPercorsi = null; renderScelta();
    if (d && p) { destSel = d.fp; inviaA(d, p); }
  },
  sceltaChiudi() { sceltaPercorsi = null; renderScelta(); },
  sceltaCoda() { coda.push(...sceltaPercorsi.map(daPercorso)); sceltaPercorsi = null; renderScelta(); vai("condividi"); },
  async idIgnora(e) { await api("identita_ignora", { fp: e.dataset.nuovo }); },
  async idSostituisci(e) {
    if (await chiedi({ titolo: `Sostituire «${e.dataset.nome}»?`, ok: "Sostituisci",
      testo: "Il nuovo dispositivo prende il posto di quello ricordato, ma NON eredita la conferma dell'identità: se vuoi, rifai la verifica con il codice." }))
      { await api("identita_sostituisci", { antico: e.dataset.antico, nuovo: e.dataset.nuovo }); toast("Dispositivo sostituito"); }
  },
};
document.addEventListener("click", ev => {
  const el = ev.target.closest("[data-azione]");
  if (el && el.dataset.azione && AZ[el.dataset.azione]) { AZ[el.dataset.azione](el, ev); return; }
  const d = ev.target.closest(".disp[data-fp]");
  if (d) { destSel = d.dataset.fp; render(); return; }
  const f = ev.target.closest("[data-file]");
  if (f) api("apri", { path: f.dataset.file });
});
document.addEventListener("keydown", e => { if (e.key === "Enter" && e.target.id === "pinCampo") document.querySelector('[data-azione="pinSi"]')?.click(); });
async function copiaTesto(t) {
  try { await navigator.clipboard.writeText(t); return; } catch (e) {}
  const a = document.createElement("textarea"); a.value = t; document.body.appendChild(a); a.select(); document.execCommand("copy"); a.remove();
}

/* impostazioni */
document.querySelectorAll("[data-conf]").forEach(i => i.addEventListener("change", async () => { await api("conf", { [i.dataset.conf]: i.checked }); }));
document.querySelectorAll("[data-conf-testo]").forEach(i => {
  const salva = async () => { const v = i.value.trim(); if (v && v !== S.conf[i.dataset.confTesto]) { await api("conf", { [i.dataset.confTesto]: v }); toast("Nome salvato"); } };
  i.addEventListener("change", salva);
  i.addEventListener("keydown", e => { if (e.key === "Enter") i.blur(); });
});
document.querySelectorAll("#segTema button").forEach(b => b.onclick = async () => { S.conf.tema = b.dataset.v; render(); await api("conf", { tema: b.dataset.v }); });
const vetro = $("#vetro");
vetro.addEventListener("input", () => { applicaAspetto.trascina = true; document.documentElement.style.setProperty("--tint", (vetro.value / 100).toFixed(2)); });
vetro.addEventListener("change", async () => { await api("conf", { vetro: +vetro.value }); S.conf.vetro = +vetro.value; applicaAspetto.trascina = false; });

$("#pinImposta").onclick = async () => {
  const v = $("#pinNuovo").value;
  const r = await api("pin", { pin: v });
  if (r.ok) { $("#pinNuovo").value = ""; toast("PIN di ricezione impostato"); } else toast(r.errore || "PIN non valido", "warn");
};
$("#pinTogli").onclick = async () => { await api("pin", { pin: "" }); toast("PIN rimosso"); };
$("#btnRipristina").onclick = async () => {
  if (await chiedi({ titolo: "Ripristinare le impostazioni?", ok: "Ripristina", pericolo: true,
    testo: "Nome, cartelle, tema, PIN, preferiti e avvio automatico tornano ai valori predefiniti. Prima viene salvata una copia in ~/.config/cable/backups. I file ricevuti e l'identità del PC non vengono toccati." })) {
    const r = await api("ripristina"); toast("Impostazioni ripristinate"); coda = [];
  }
};
$("#btnIdentita").onclick = async () => {
  if (await chiedi({ titolo: "Rigenerare l'identità del PC?", ok: "Rigenera", pericolo: true,
    testo: "Gli altri dispositivi vedranno questo PC come un dispositivo nuovo e dovranno ricollegarlo. I vecchi certificato e chiave restano in una cartella di backup. I file non vengono toccati." })) {
    const r = await api("rigenera_identita"); toast(r.ok ? "Nuova identità creata" : "Fatto, ma la ricezione non è ripartita", r.ok ? "ok" : "warn");
  }
};
$("#btnDiag").onclick = async () => { const r = await api("diagnostica"); $("#diagTesto").textContent = r.testo || "Non disponibile"; };

/* testo */
$("#homeIncolla").onclick = async () => {
  try { $("#homeTesto").value = await navigator.clipboard.readText(); } catch (e) { $("#homeTesto").focus(); toast("Premi Ctrl+V per incollare", "warn"); }
};
$("#homeInviaTesto").onclick = async () => {
  const t = $("#homeTesto").value; const d = destinazione();
  if (!t.trim()) { $("#homeTesto").focus(); return; }
  if (!d) { toast(nessunDest(), "warn"); return; }
  if (await inviaA(d, [], t)) $("#homeTesto").value = "";
};

/* cerca */
$("#cerca").onclick = () => AZ.aggiorna();
$("#ipVai").onclick = async () => {
  const ip = $("#ipManuale").value.trim();
  if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(ip)) { toast("Scrivi un indirizzo come 192.168.1.50", "warn"); return; }
  const r = await api("cerca", { ip }); toast(r.trovati ? "Collegato" : "Non risponde: LocalSend è aperto sul telefono?", r.trovati ? "ok" : "warn");
};

/* ------------------------------------------------------------ file: scegli e trascina */
$("#homeDrop").onclick = () => scegliFile(true);
$("#condDrop").onclick = () => scegliFile(false);
$("#inviaCoda").onclick = async () => {
  const d = destinazione(); if (!d || !coda.length) return;
  if (await inviaA(d, coda.map(c => c.path))) coda = [];
};
/* il trascinamento è gestito dalla finestra GTK, che consegna i percorsi reali (nessuna copia) */
window.hubDragState = on => {
  const d = destinazione();
  $("#rilascioT").textContent = d ? `Rilascia per inviare a ${d.alias}` : "Rilascia per aggiungerli";
  $("#rilascioS").textContent = d ? "oppure lasciali sul dispositivo che preferisci in Condividi" : nessunDest();
  $("#rilascio").classList.toggle("on", !!on);
  if (!on) document.querySelectorAll(".disp.sopra").forEach(x => x.classList.remove("sopra"));
};
window.hubDrop = (percorsi, x, y) => {
  const su = document.elementFromPoint(x, y)?.closest?.(".disp[data-fp]");
  if (su) { const d = vicini().find(v => v.fp === su.dataset.fp); if (d) { accogli(percorsi, true, d); return; } }
  chiediDestinatario(percorsi);                      // rilascio sulla finestra: scelta del destinatario
};
window.hubFile = percorsi => chiediDestinatario(percorsi);   // file passati da riga di comando / file manager

/* Richieste di apertura (CLI, file manager, seconda istanza): la coda è nel backend. L'evento «hub-apertura» è
   solo una sveglia; si ritirano le richieste, le si mostra, poi le si conferma per id. Un id già visto non
   viene ripresentato anche se la conferma è ancora in volo; una richiesta mai confermata torna alla pagina
   successiva. Niente parte da sola: si arriva sempre alla scelta del destinatario. */
const aperteViste = new Set();
let aperturaInCorso = false, aperturaDaRifare = false;
async function ritiraAperture() {
  if (aperturaInCorso) { aperturaDaRifare = true; return; }
  aperturaInCorso = true;
  try {
    do {
      aperturaDaRifare = false;
      const reqs = await TAURI.core.invoke("richieste_apertura");
      for (const r of reqs) {
        if (aperteViste.has(r.id)) continue;
        aperteViste.add(r.id);
        if (r.paths.length) window.hubFile(r.paths);
        if (r.mancanti.length) toast(r.mancanti.length === 1 ? `File non trovato: ${r.mancanti[0].split("/").pop()}` : `${r.mancanti.length} file non trovati`, "warn");
      }
      if (reqs.length) await TAURI.core.invoke("conferma_apertura", { ids: reqs.map(r => r.id) });
    } while (aperturaDaRifare);
  } catch (e) { api("js_error", { m: "aperture: " + e }); }
  finally { aperturaInCorso = false; }
}
window.ritiraAperture = ritiraAperture;
// trascinamento dall'app verso il desktop
document.addEventListener("dragstart", e => {
  const f = e.target.closest?.("[data-file]"); if (!f) return;
  const uri = "file://" + f.dataset.file.split("/").map(encodeURIComponent).join("/");
  e.dataTransfer.effectAllowed = "copy";
  e.dataTransfer.setData("text/uri-list", uri);
  e.dataTransfer.setData("text/plain", f.dataset.file);
});


/* ------------------------------------------------------------ wizard di primo collegamento */
/* Guidato e sempre saltabile: non blocca i trasferimenti. Verifica con un codice inviato via LocalSend
   (connessione con impronta controllata): prova che l'impronta appartiene al telefono che hai in mano. */
let wizOpen = false, wizDismissed = false, wizStep = 0, wizSel = null, wizKey = "", wizProvaAvviata = false, wizInizio = 0;
const WIZ = ["Prepara", "Trova", "Verifica", "Prova", "Fine"];
const wizVisible = () => !!S && (wizOpen || (S.conf.wizard_fatto === false && !wizDismissed));

function wizTemplate(step) {
  const dots = `<div class="wdots">${WIZ.map((n, i) => `<div class="wdot ${i === step ? "on" : i < step ? "fatto" : ""}"><i>${i + 1}</i>${n}</div>`).join("")}</div>`;
  const foot = `<div class="wfoot"><button class="btn" data-azione="wizSalta">${step === 4 ? "Chiudi" : "Salta per ora"}</button><span class="grow"></span>
    ${step > 0 ? '<button class="btn" data-azione="wizIndietro">Indietro</button>' : ""}
    <button class="btn primario" id="wizNext" data-azione="wizAvanti">${step === 4 ? "Inizia a usare Cable" : "Avanti"}</button></div>`;
  const body = [
    `<h2>Prepara lo smartphone</h2><p>Cable usa il protocollo LocalSend: sullo smartphone (Android o iPhone) serve l'app LocalSend o una compatibile. Nessun account, tutto resta sulla tua rete.</p>
     <div class="wcheck" id="wizChecks"></div>
     <div class="passi">
       <div class="passo"><div class="n">1</div><div><b>Apri LocalSend sullo smartphone</b><small>Se non ce l'hai, installala dallo store del tuo sistema</small></div></div>
       <div class="passo"><div class="n">2</div><div><b>Stessa Wi-Fi del PC</b><small>Va bene anche l'hotspot del telefono</small></div></div>
       <div class="passo"><div class="n">3</div><div><b>Lascia attiva la cifratura (HTTPS)</b><small>Cable non parla con dispositivi senza cifratura</small></div></div>
       <div class="passo"><div class="n">4</div><div><b>Tieni aperta la scheda «Ricevi»</b><small>Così lo smartphone è visibile</small></div></div>
     </div>`,
    `<h2>Scegli lo smartphone</h2><p>Compaiono i dispositivi vicini. Scegli tu quello giusto: Cable non sceglie mai al posto tuo.</p>
     <div class="vicini wbox" id="wizDevs" style="min-height:130px"></div>
     <details><summary>Non lo vedi? Inserisci l'indirizzo IP</summary>
       <div style="display:flex;gap:10px;margin-top:10px"><input class="input" id="wizIp" placeholder="es. 192.168.1.50 (lo trovi in LocalSend › Ricevi)" style="flex:1"><button class="btn" id="wizIpVai">Collega</button></div></details>`,
    `<h2>Verifica con un codice (facoltativa)</h2><p>Il PC invia un codice allo smartphone scelto. Inserendolo qui dimostri che stai parlando con <b>quel</b> dispositivo e non con un altro in rete.</p>
     <div class="wbox"><div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><button class="btn primario" id="wizInvia">Invia codice al telefono</button><span id="wizStato" class="vuoto"></span></div>
       <p class="vuoto" style="margin-top:10px;font-size:13.5px">Sul telefono compare un messaggio «Cable — codice di verifica». Se LocalSend lo chiede, tocca <b>Accetta</b>.</p></div>
     <div class="wbox"><label style="display:block;font-size:13.5px;color:var(--txt2);margin-bottom:8px">Codice di 6 cifre ricevuto sul telefono</label>
       <input class="input wcode" id="wizCode" inputmode="numeric" maxlength="6" autocomplete="off" placeholder="——————">
       <div style="display:flex;gap:10px;margin-top:12px;align-items:center"><button class="btn primario" id="wizVerifica">Verifica</button><span id="wizCount" class="vuoto"></span></div>
       <div class="wmsg" id="wizMsg"></div></div>`,
    `<h2>Prova dal telefono</h2><p>Ora l'altro verso: dal telefono manda qualcosa al PC, così sappiamo che ricevere funziona (rete e firewall).</p>
     <div class="passi">
       <div class="passo"><div class="n">1</div><div><b>In LocalSend tocca «Invia»</b><small>Scegli un testo o un file qualsiasi</small></div></div>
       <div class="passo"><div class="n">2</div><div><b>Seleziona questo PC: «<span id="wizNome"></span>»</b><small>Compare nell'elenco dei dispositivi vicini</small></div></div>
       <div class="passo"><div class="n">3</div><div><b>Accetta qui la richiesta</b><small>Ogni ricezione chiede sempre conferma</small></div></div>
     </div>
     <div class="wbox" id="wizProva"></div>`,
    `<h2>Tutto pronto</h2><p>Ultimi ritocchi: puoi cambiarli quando vuoi da Impostazioni.</p>
     <div class="vetro lista" style="margin:14px 0;background:var(--riempi);box-shadow:none">
       <div class="riga"><div class="testo"><b>Nome di questo PC</b><small>Come lo vedono gli altri dispositivi</small></div><input class="input" id="wizNomePc" maxlength="40" style="width:200px;text-align:left"></div>
       <div class="riga"><div class="testo"><b>Cartella dei file ricevuti</b><small id="wizCartella"></small></div><button class="btn piccolo" data-azione="wizCartella">Cambia</button></div>
       <div class="riga"><div class="testo"><b>PIN di ricezione (consigliato)</b><small>Chi ti invia qualcosa dovrà inserirlo. Facoltativo, 4–32 caratteri</small></div><input class="input" id="wizPin" type="password" maxlength="32" autocomplete="new-password" style="width:140px;text-align:left" placeholder="nessun PIN"></div>
       <div class="riga"><div class="testo"><b>Avvia all'accensione</b><small>Parte nascosto, pronto a ricevere</small></div><label class="sw"><input type="checkbox" id="wizAvvio"><span></span></label></div>
     </div>`,
  ][step];
  return dots + body + foot;
}

function renderWizard() {
  const el = $("#wizard"), vis = wizVisible();
  el.classList.toggle("on", vis);
  $("#main").inert = vis; $("#tabbar").inert = vis;
  if (!vis) { wizKey = ""; return; }
  if (wizKey !== String(wizStep)) {
    wizKey = String(wizStep);
    $("#wizCard").innerHTML = wizTemplate(wizStep);
    icone($("#wizCard"));
    wizBind();
    if (wizStep === 3 && wizSel) { wizInizio = Date.now(); api("wizard_prova", { fp: wizSel }); }
  }
  wizUpdate();
}

const wizDev = () => (S.dispositivi || []).find(d => d.fp === wizSel);
const wizOk = () => !!(S.wizard && S.wizard.confermato_fp && S.wizard.confermato_fp === wizSel);

function wizBind() {
  const on = (id, ev, fn) => { const e = $(id); if (e) e.addEventListener(ev, fn); };
  on("#wizIpVai", "click", async () => {
    const ip = $("#wizIp").value.trim();
    if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(ip)) { toast("Scrivi un indirizzo come 192.168.1.50", "warn"); return; }
    const r = await api("cerca", { ip }); toast(r.trovati ? "Trovato" : "Non risponde: LocalSend è aperto sul telefono?", r.trovati ? "ok" : "warn");
  });
  on("#wizInvia", "click", async () => {
    $("#wizMsg").textContent = ""; $("#wizMsg").className = "wmsg";
    const r = await api("wizard_verifica_inizia", { fp: wizSel });
    if (!r.ok) { $("#wizMsg").textContent = r.errore || "Non riesco a inviare il codice."; $("#wizMsg").className = "wmsg err"; }
  });
  const verifica = async () => {
    const r = await api("wizard_verifica_conferma", { code: $("#wizCode").value });
    const m = $("#wizMsg");
    if (r.ok) { m.textContent = `Verificato: «${r.alias}» è il tuo telefono.`; m.className = "wmsg ok"; $("#wizCode").value = ""; }
    else { m.textContent = r.errore || "Codice non valido."; m.className = "wmsg err"; }
  };
  on("#wizVerifica", "click", verifica);
  on("#wizCode", "keydown", e => { if (e.key === "Enter") verifica(); });
  on("#wizNomePc", "change", async e => { const v = e.target.value.trim(); if (v) { await api("conf", { nome_pc: v }); } });
  on("#wizAvvio", "change", async e => { await api("conf", { avvio_login: e.target.checked }); });
  on("#wizPin", "change", async e => {
    const r = await api("pin", { pin: e.target.value });
    if (r.ok) toast(e.target.value ? "PIN di ricezione impostato" : "Nessun PIN"); else toast(r.errore || "PIN non valido", "warn");
  });
}

function wizUpdate() {
  const c = S.conf;
  if (wizStep === 0) {
    const ok = (b, t, extra = "") => `<div><span style="color:var(--${b ? "verde" : "arancio"})">${b ? I.ok : I.attenzione}</span><span>${t}</span>${extra}</div>`;
    const id = S.identita;
    set("#wizChecks", ok(S.rete.ip.length > 0, S.rete.ip.length ? `Il PC è in rete <small>${esc(S.rete.ip.join(", "))}</small>` : "Il PC non è collegato a nessuna rete")
      + ok(S.server.attivo, S.server.attivo ? "Ricezione attiva <small>porta " + S.server.porta + ", connessione cifrata</small>" : (S.server.errore === "porta" ? "La porta " + S.server.porta + " è occupata <small>di solito da LocalSend per PC: chiudilo</small>" : "Ricezione non attiva"),
          S.server.attivo ? "" : `<button class="btn piccolo" data-azione="riprovaLS">Riprova</button>`)
      + ok(id.stato === "ok", id.stato === "ok" ? "Identità di questo PC valida" : "Identità non valida <small>" + esc(id.errore) + "</small>"));
  }
  if (wizStep === 1) {
    const v = vicini();
    if (wizSel && !v.find(d => d.fp === wizSel)) { /* scomparso: resta selezionato finché non scegli altro */ }
    set("#wizDevs", v.map(d => `<div class="disp ${d.fp === wizSel ? "sel" : ""}" data-azione="wizScegli" data-fp="${esc(d.fp)}" title="${esc(d.ip)}">
        <div class="av">${d.tipo === "desktop" ? I.pc : esc(iniziale(d.alias))}</div><b>${esc(d.alias)}</b><small>${esc(d.modello || d.ip)}</small></div>`).join("")
      + `<div class="disp" style="cursor:default"><div class="radar">${I.telefono}</div><b>${v.length ? "Cerco altri…" : "Cerco…"}</b><small>${v.length ? "" : "Apri LocalSend sul telefono"}</small></div>`);
  }
  if (wizStep === 2) {
    const w = S.wizard, d = wizDev(), ch = w.sfida || {}, inv = w.invio;
    const ok = wizOk();
    let st = "";
    if (ok) st = "";
    else if (inv && inv.stato === "attesa") st = "Aspetto che il telefono mostri il messaggio…";
    else if (inv && inv.stato === "completato") st = "Codice consegnato: leggilo sul telefono.";
    else if (inv && ["errore", "rifiutato", "annullato"].includes(inv.stato)) st = "Non consegnato: " + (inv.errore || inv.stato);
    else if (inv) st = "Invio in corso…";
    $("#wizStato").textContent = st;
    $("#wizInvia").textContent = inv ? "Invia di nuovo" : "Invia codice al telefono";
    $("#wizInvia").disabled = ok || !d;
    $("#wizVerifica").disabled = ok || !ch.attivo;
    $("#wizCode").disabled = ok || !ch.attivo;
    $("#wizCount").textContent = ok ? "" : ch.attivo ? `Scade fra ${Math.floor(ch.secondi / 60)}:${String(ch.secondi % 60).padStart(2, "0")} · ${ch.tentativi} tentativi` : "";
    if (ok) { const m = $("#wizMsg"); if (!m.textContent) { m.textContent = `Verificato: «${(d && d.alias) || "il telefono"}» è il tuo telefono.`; m.className = "wmsg ok"; } }
  }
  if (wizStep === 3) {
    const rx = S.wizard.prova_ricevuta;
    $("#wizNome").textContent = c.nome_pc;
    const lenta = Date.now() - wizInizio > 30000;
    set("#wizProva", rx
      ? `<div style="display:flex;gap:12px;align-items:center;color:var(--verde)">${I.ok}<b>Ricezione funzionante: è arrivato qualcosa dal telefono.</b></div>`
      : `<div style="display:flex;gap:14px;align-items:center"><div class="radar" style="width:46px;height:46px">${I.telefono}</div><div><b>In attesa di qualcosa dal telefono…</b>
         ${lenta ? `<p class="vuoto" style="font-size:13.5px;margin-top:6px">Non arriva? Controlla che i due dispositivi siano sulla stessa rete e guarda <span class="link" data-azione="wizDiag">la diagnostica</span> (il firewall è solo una delle possibili cause). Puoi anche saltare questa prova.</p>` : ""}</div></div>`);
  }
  if (wizStep === 4) {
    const act = document.activeElement;
    if (act !== $("#wizNomePc")) $("#wizNomePc").value = c.nome_pc;
    set("#wizCartella", esc(c.cartella));
    $("#wizAvvio").checked = !!c.avvio_login;
  }
  // il pulsante «Avanti» si abilita solo quando il passo è davvero concluso
  const next = $("#wizNext");
  if (next) next.disabled = (wizStep === 1 && !(wizSel && vicini().find(d => d.fp === wizSel))) || (wizStep === 2 && !wizOk());
}

Object.assign(AZ, {
  wizApri() { wizOpen = true; wizDismissed = false; wizStep = 0; wizSel = null; wizKey = ""; render(); },
  async wizSalta() { await api("wizard_completa"); wizOpen = false; wizDismissed = true; wizKey = ""; render(); toast("Puoi collegare un telefono quando vuoi da Impostazioni"); },
  wizIndietro() { if (wizStep > 0) { wizStep--; renderWizard(); } },
  async wizAvanti() {
    if (wizStep < 4) { wizStep++; renderWizard(); return; }
    const pin = $("#wizPin").value;
    if (pin) { const r = await api("pin", { pin }); if (!r.ok) { toast(r.errore || "PIN non valido", "warn"); return; } }
    await api("wizard_completa"); wizOpen = false; wizDismissed = true; wizKey = "";
    if (wizSel) destSel = wizSel;
    render(); toast("Telefono collegato");
  },
  wizScegli(e) { wizSel = e.dataset.fp; wizUpdate(); },
  async wizCartella() { const r = await api("scegli_cartella", { quale: "file" }); if (r.ok) toast("Cartella aggiornata"); },
  wizDiag() { wizOpen = false; wizDismissed = true; wizKey = ""; vai("impostazioni"); render(); },
});

/* ------------------------------------------------------------ stato: spinto dal motore, nessun polling */
window.hubStato = s => { S = s; render(); };
window.__errs = 0;
window.addEventListener("error", e => { window.__errs++; api("js_error", { m: String(e.message), l: e.lineno }); });
window.addEventListener("unhandledrejection", e => { window.__errs++; api("js_error", { m: String(e.reason) }); });
TAURI.event.listen("hub-stato", e => window.hubStato(e.payload));
TAURI.event.listen("hub-drag", e => window.hubDragState(e.payload));
TAURI.event.listen("hub-drop", e => window.hubDrop(e.payload.paths, e.payload.x, e.payload.y));
TAURI.event.listen("hub-apertura", () => ritiraAperture());
icone();
try { const p = localStorage.getItem("hub.pagina"); if (p && document.querySelector(`.pagina[data-p="${p}"]`)) vai(p); } catch (e) {}
api("stato").then(s => { if (s && s.versione) window.hubStato(s); });
ritiraAperture();
document.addEventListener("keydown", e => {
  if (e.ctrlKey && /^[1-5]$/.test(e.key)) vai(["home", "schermo", "condividi", "pixel", "impostazioni"][+e.key - 1]);
});
