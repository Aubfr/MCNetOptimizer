//! Implementation Windows (FFI direct vers iphlpapi, wlanapi, kernel32, user32, shell32).
//!
//! Les declarations FFI sont locales pour eviter une dependance a une version precise
//! de la crate `windows`. Chaque bloc `unsafe` est court et documente.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::time::Instant;

use crate::model::WifiInfo;
use crate::parse;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// Declarations FFI
// ---------------------------------------------------------------------------

#[link(name = "iphlpapi")]
extern "system" {
    fn IcmpCreateFile() -> isize;
    fn IcmpCloseHandle(handle: isize) -> i32;
    fn IcmpSendEcho(
        handle: isize,
        dest: u32,
        request_data: *const c_void,
        request_size: u16,
        request_options: *const c_void,
        reply_buffer: *mut c_void,
        reply_size: u32,
        timeout_ms: u32,
    ) -> u32;
    fn GetExtendedTcpTable(
        table: *mut c_void,
        size: *mut u32,
        order: i32,
        af: u32,
        table_class: u32,
        reserved: u32,
    ) -> u32;
    fn SetPerTcpConnectionEStats(
        row: *mut c_void,
        estats_type: u32,
        rw: *const u8,
        rw_version: u32,
        rw_size: u32,
        offset: u32,
    ) -> u32;
    fn GetPerTcpConnectionEStats(
        row: *mut c_void,
        estats_type: u32,
        rw: *mut u8,
        rw_version: u32,
        rw_size: u32,
        ros: *mut u8,
        ros_version: u32,
        ros_size: u32,
        rod: *mut u8,
        rod_version: u32,
        rod_size: u32,
    ) -> u32;
    fn GetIfTable(table: *mut c_void, size: *mut u32, order: i32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn GetPriorityClass(process: isize) -> u32;
    fn SetPriorityClass(process: isize, class: u32) -> i32;
    fn GetLastError() -> u32;
    fn Beep(freq: u32, duration_ms: u32) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(hwnd: isize, id: i32, modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: isize, id: i32) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn IsUserAnAdmin() -> i32;
    fn ShellExecuteW(
        hwnd: isize,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

#[link(name = "wlanapi")]
extern "system" {
    fn WlanOpenHandle(version: u32, reserved: *mut c_void, negotiated: *mut u32, handle: *mut isize) -> u32;
    fn WlanCloseHandle(handle: isize, reserved: *mut c_void) -> u32;
    fn WlanEnumInterfaces(handle: isize, reserved: *mut c_void, list: *mut *mut c_void) -> u32;
    fn WlanQueryInterface(
        handle: isize,
        guid: *const c_void,
        opcode: u32,
        reserved: *mut c_void,
        data_size: *mut u32,
        data: *mut *mut c_void,
        value_type: *mut u32,
    ) -> u32;
    fn WlanFreeMemory(ptr: *mut c_void);
}

#[repr(C)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; 260],
}

#[repr(C)]
struct Msg {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt_x: i32,
    pt_y: i32,
    l_private: u32,
}

// ---------------------------------------------------------------------------
// Execution de commandes systeme (sans fenetre)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Lance un programme sans fenetre console et capture sa sortie.
pub fn run_hidden(program: &str, args: &[&str]) -> Result<CmdOutput, String> {
    let out = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("{}: {}", program, e))?;
    Ok(CmdOutput {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

/// Lance un script PowerShell et renvoie sa sortie standard (Err si code != 0).
pub fn powershell(script: &str) -> Result<String, String> {
    let out = run_hidden(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
    )?;
    if out.code == 0 {
        Ok(out.stdout.trim().to_string())
    } else {
        let msg = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
        Err(format!("PowerShell (code {}): {}", out.code, msg))
    }
}

/// Passerelle IPv4 par defaut.
pub fn default_gateway() -> Option<Ipv4Addr> {
    let out = run_hidden("route", &["print", "-4"]).ok()?;
    parse::parse_route_print(&out.stdout)
}

pub fn is_admin() -> bool {
    // SAFETY: appel sans argument, renvoie un booleen.
    unsafe { IsUserAnAdmin() != 0 }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Relance l'executable courant avec elevation (invite UAC). true si la demande est lancee.
pub fn relaunch_as_admin() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let op = wide("runas");
    let file = wide(&exe.to_string_lossy());
    // SAFETY: chaines UTF-16 terminees par 0, valides pendant l'appel.
    let r = unsafe { ShellExecuteW(0, op.as_ptr(), file.as_ptr(), null(), null(), 1) };
    r > 32
}

pub fn beep(freq: u32, ms: u32) {
    // SAFETY: appel simple.
    unsafe {
        Beep(freq, ms);
    }
}

// ---------------------------------------------------------------------------
// ICMP
// ---------------------------------------------------------------------------

pub struct IcmpPinger {
    handle: isize,
}

impl IcmpPinger {
    pub fn new() -> Option<IcmpPinger> {
        // SAFETY: IcmpCreateFile ne prend aucun argument.
        let h = unsafe { IcmpCreateFile() };
        if h == -1 || h == 0 {
            None
        } else {
            Some(IcmpPinger { handle: h })
        }
    }

    /// RTT en ms (mesure autour de l'appel, resolution sub-milliseconde), None si perte.
    pub fn ping(&self, ip: Ipv4Addr, timeout_ms: u32) -> Option<f32> {
        let addr = u32::from_le_bytes(ip.octets());
        let payload = [0x61u8; 32];
        let mut reply = [0u8; 256];
        let start = Instant::now();
        // SAFETY: tampons valides et de tailles annoncees ; handle valide jusqu'au Drop.
        let n = unsafe {
            IcmpSendEcho(
                self.handle,
                addr,
                payload.as_ptr() as *const c_void,
                payload.len() as u16,
                null(),
                reply.as_mut_ptr() as *mut c_void,
                reply.len() as u32,
                timeout_ms,
            )
        };
        let elapsed = start.elapsed().as_secs_f32() * 1000.0;
        if n == 0 {
            return None;
        }
        // ICMP_ECHO_REPLY : Address (u32), Status (u32), RoundTripTime (u32)...
        let status = u32::from_le_bytes([reply[4], reply[5], reply[6], reply[7]]);
        if status == 0 {
            Some(elapsed)
        } else {
            None
        }
    }
}

impl Drop for IcmpPinger {
    fn drop(&mut self) {
        // SAFETY: handle cree par IcmpCreateFile.
        unsafe {
            IcmpCloseHandle(self.handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Processus
// ---------------------------------------------------------------------------

/// PID des processus dont le nom d'executable (insensible a la casse) figure dans `names`.
pub fn find_pids(names: &[&str]) -> Vec<u32> {
    let mut out = Vec::new();
    // SAFETY: snapshot de processus en lecture seule ; la structure est initialisee avec dw_size.
    unsafe {
        let snap = CreateToolhelp32Snapshot(2, 0);
        if snap == -1 || snap == 0 {
            return out;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            let len = entry.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&entry.sz_exe_file[..len]).to_lowercase();
            if names.iter().any(|n| n.to_lowercase() == name) {
                out.push(entry.th32_process_id);
            }
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
    }
    out
}

const PROCESS_SET_INFORMATION: u32 = 0x0200;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

pub const PRIORITY_NORMAL: u32 = 0x0000_0020;
pub const PRIORITY_ABOVE_NORMAL: u32 = 0x0000_8000;

pub fn get_priority_class(pid: u32) -> Option<u32> {
    // SAFETY: OpenProcess avec droits minimaux, handle ferme ensuite.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return None;
        }
        let c = GetPriorityClass(h);
        CloseHandle(h);
        if c == 0 {
            None
        } else {
            Some(c)
        }
    }
}

pub fn set_priority_class(pid: u32, class: u32) -> Result<(), String> {
    // SAFETY: OpenProcess avec droit de modifier uniquement la priorite.
    unsafe {
        let h = OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return Err(format!("OpenProcess a échoué (code {})", GetLastError()));
        }
        let ok = SetPriorityClass(h, class);
        let err = GetLastError();
        CloseHandle(h);
        if ok == 0 {
            Err(format!("SetPriorityClass a échoué (code {})", err))
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Table TCP et statistiques de connexion (EStats)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TcpConn {
    pub state: u32,
    pub local: SocketAddrV4,
    pub remote: SocketAddrV4,
    pub pid: u32,
    /// MIB_TCPROW : state, local addr, local port, remote addr, remote port.
    pub(crate) row: [u32; 5],
}

impl TcpConn {
    pub fn is_established(&self) -> bool {
        self.state == 5
    }
}

fn rd_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn port_from_raw(v: u32) -> u16 {
    u16::from_be((v & 0xFFFF) as u16)
}

fn addr_from_raw(v: u32) -> Ipv4Addr {
    Ipv4Addr::from(v.to_le_bytes())
}

/// Connexions TCP IPv4 avec PID proprietaire (TCP_TABLE_OWNER_PID_ALL).
pub fn tcp_table() -> Result<Vec<TcpConn>, u32> {
    let mut size: u32 = 0;
    let mut buf: Vec<u8> = Vec::new();
    let mut ok = false;
    for _ in 0..6 {
        // SAFETY: buf a la taille annoncee dans size (ou est vide avec size = 0).
        let ret = unsafe {
            GetExtendedTcpTable(
                if buf.is_empty() { null_mut() } else { buf.as_mut_ptr() as *mut c_void },
                &mut size,
                0,
                2,
                5,
                0,
            )
        };
        if ret == 0 {
            ok = true;
            break;
        }
        if ret == 122 {
            buf = vec![0u8; size as usize + 2048];
            size = buf.len() as u32;
            continue;
        }
        return Err(ret);
    }
    if !ok {
        return Err(122);
    }
    if buf.len() < 4 {
        return Ok(Vec::new());
    }
    let n = rd_u32(&buf, 0) as usize;
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let off = 4 + k * 24;
        if off + 24 > buf.len() {
            break;
        }
        let state = rd_u32(&buf, off);
        let la = rd_u32(&buf, off + 4);
        let lp = rd_u32(&buf, off + 8);
        let ra = rd_u32(&buf, off + 12);
        let rp = rd_u32(&buf, off + 16);
        let pid = rd_u32(&buf, off + 20);
        out.push(TcpConn {
            state,
            local: SocketAddrV4::new(addr_from_raw(la), port_from_raw(lp)),
            remote: SocketAddrV4::new(addr_from_raw(ra), port_from_raw(rp)),
            pid,
            row: [state, la, lp, ra, rp],
        });
    }
    Ok(out)
}

/// Compteurs reels d'une connexion TCP (TcpConnectionEstatsPath).
#[derive(Clone, Copy, Debug, Default)]
pub struct PathStats {
    pub sample_rtt_ms: u32,
    pub smoothed_rtt_ms: u32,
    pub min_rtt_ms: u32,
    pub max_rtt_ms: u32,
    pub pkts_retrans: u32,
    pub timeouts: u32,
}

const ESTATS_PATH: u32 = 3;

/// Active la collecte "Path" pour une connexion (droits administrateur requis).
pub fn estats_enable(conn: &TcpConn) -> Result<(), u32> {
    let rw: [u8; 1] = [1];
    let mut row = conn.row;
    // SAFETY: row = MIB_TCPROW (5 DWORD) ; rw = TCP_ESTATS_PATH_RW_v0 (1 octet).
    let ret = unsafe { SetPerTcpConnectionEStats(row.as_mut_ptr() as *mut c_void, ESTATS_PATH, rw.as_ptr(), 0, 1, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(ret)
    }
}

/// Lit les compteurs "Path" d'une connexion. Valide la coherence des valeurs lues :
/// en cas d'incoherence (structure differente selon la version de Windows) renvoie Err(0).
pub fn estats_read(conn: &TcpConn) -> Result<PathStats, u32> {
    let mut row = conn.row;
    let mut last_err = 0u32;
    // La structure TCP_ESTATS_PATH_ROD_v0 compte 40 ULONG (160 octets) ; on essaie aussi
    // une taille plus grande au cas ou une version de Windows en ajouterait.
    for size_bytes in [160usize, 192usize] {
        let mut rod = [0u32; 48];
        // SAFETY: rod fait 192 octets >= size_bytes ; row = MIB_TCPROW.
        let ret = unsafe {
            GetPerTcpConnectionEStats(
                row.as_mut_ptr() as *mut c_void,
                ESTATS_PATH,
                null_mut(),
                0,
                0,
                null_mut(),
                0,
                0,
                rod.as_mut_ptr() as *mut u8,
                0,
                size_bytes as u32,
            )
        };
        if ret != 0 {
            last_err = ret;
            continue;
        }
        let st = PathStats {
            timeouts: rod[1],
            pkts_retrans: rod[5],
            sample_rtt_ms: rod[26],
            smoothed_rtt_ms: rod[27],
            max_rtt_ms: rod[29],
            min_rtt_ms: rod[30],
        };
        // Coherence : RTT lisses plausibles (0 < x < 10 s) et min <= max.
        let plausible = st.smoothed_rtt_ms > 0
            && st.smoothed_rtt_ms < 10_000
            && st.sample_rtt_ms < 10_000
            && st.min_rtt_ms <= st.max_rtt_ms.max(st.smoothed_rtt_ms);
        if plausible {
            return Ok(st);
        }
        last_err = 0;
    }
    Err(last_err)
}

// ---------------------------------------------------------------------------
// Compteurs d'interfaces (charge reseau locale)
// ---------------------------------------------------------------------------

/// (index d'interface, octets recus, octets emis) pour chaque interface active hors loopback.
/// Les compteurs sont sur 32 bits : l'appelant doit utiliser wrapping_sub.
pub fn if_octets() -> Vec<(u32, u32, u32)> {
    let mut size: u32 = 0;
    let mut buf: Vec<u8> = Vec::new();
    let mut ok = false;
    for _ in 0..6 {
        // SAFETY: meme schema que tcp_table.
        let ret = unsafe {
            GetIfTable(
                if buf.is_empty() { null_mut() } else { buf.as_mut_ptr() as *mut c_void },
                &mut size,
                0,
            )
        };
        if ret == 0 {
            ok = true;
            break;
        }
        if ret == 122 {
            buf = vec![0u8; size as usize + 4096];
            size = buf.len() as u32;
            continue;
        }
        return Vec::new();
    }
    if !ok || buf.len() < 4 {
        return Vec::new();
    }
    const ROW: usize = 860;
    let n = rd_u32(&buf, 0) as usize;
    let mut out = Vec::new();
    for k in 0..n {
        let off = 4 + k * ROW;
        if off + ROW > buf.len() {
            break;
        }
        let index = rd_u32(&buf, off + 512);
        let itype = rd_u32(&buf, off + 516);
        let oper = rd_u32(&buf, off + 544);
        let inb = rd_u32(&buf, off + 552);
        let outb = rd_u32(&buf, off + 576);
        // 24 = loopback logiciel ; etat >= 4 : connecte / operationnel.
        if itype != 24 && oper >= 4 {
            out.push((index, inb, outb));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Wi-Fi (API WLAN native, independante de la langue de Windows)
// ---------------------------------------------------------------------------

#[repr(C)]
struct WlanInterfaceInfo {
    guid: [u8; 16],
    description: [u16; 256],
    state: u32,
}

#[repr(C)]
struct WlanAssociation {
    ssid_len: u32,
    ssid: [u8; 32],
    bss_type: u32,
    bssid: [u8; 6],
    phy_type: u32,
    phy_index: u32,
    signal_quality: u32,
    rx_rate_kbps: u32,
    tx_rate_kbps: u32,
}

/// Prefixe de WLAN_CONNECTION_ATTRIBUTES (la partie securite n'est pas lue).
#[repr(C)]
struct WlanConnectionHead {
    state: u32,
    mode: u32,
    profile: [u16; 256],
    association: WlanAssociation,
}

fn phy_name(t: u32) -> String {
    match t {
        3 => "802.11a",
        4 => "802.11b",
        5 => "802.11g",
        7 => "Wi-Fi 4 (802.11n)",
        8 => "Wi-Fi 5 (802.11ac)",
        10 => "Wi-Fi 6 (802.11ax)",
        11 => "Wi-Fi 7 (802.11be)",
        _ => "Wi-Fi",
    }
    .to_string()
}

pub fn band_from_channel(ch: u32) -> &'static str {
    if ch == 0 {
        "?"
    } else if ch <= 14 {
        "2,4 GHz"
    } else {
        "5/6 GHz"
    }
}

/// Etat de la liaison Wi-Fi courante, None si aucune interface Wi-Fi connectee.
pub fn wlan_current() -> Option<WifiInfo> {
    let mut negotiated = 0u32;
    let mut handle: isize = 0;
    // SAFETY: parametres de sortie valides ; handle ferme avant de retourner.
    unsafe {
        if WlanOpenHandle(2, null_mut(), &mut negotiated, &mut handle) != 0 {
            return None;
        }
        let r = wlan_query_connected(handle);
        WlanCloseHandle(handle, null_mut());
        r
    }
}

unsafe fn wlan_query_connected(handle: isize) -> Option<WifiInfo> {
    let mut list: *mut c_void = null_mut();
    if WlanEnumInterfaces(handle, null_mut(), &mut list) != 0 || list.is_null() {
        return None;
    }
    let base = list as *const u8;
    let count = *(base as *const u32) as usize;
    let items = base.add(8) as *const WlanInterfaceInfo;
    let mut result: Option<WifiInfo> = None;
    for i in 0..count {
        let info = &*items.add(i);
        if info.state != 1 {
            continue; // 1 = wlan_interface_state_connected
        }
        let guid_ptr = info.guid.as_ptr() as *const c_void;

        let mut size = 0u32;
        let mut data: *mut c_void = null_mut();
        let mut vt = 0u32;
        let r = WlanQueryInterface(handle, guid_ptr, 7, null_mut(), &mut size, &mut data, &mut vt);
        if r != 0 || data.is_null() {
            continue;
        }
        if (size as usize) < std::mem::size_of::<WlanConnectionHead>() {
            WlanFreeMemory(data);
            continue;
        }
        let head = &*(data as *const WlanConnectionHead);
        let a = &head.association;
        let ssid_len = (a.ssid_len as usize).min(32);
        let ssid = String::from_utf8_lossy(&a.ssid[..ssid_len]).to_string();
        let bssid = a
            .bssid
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(":");
        let quality = a.signal_quality;
        let rx = a.rx_rate_kbps as f32 / 1000.0;
        let tx = a.tx_rate_kbps as f32 / 1000.0;
        let phy = phy_name(a.phy_type);
        WlanFreeMemory(data);

        // Canal (opcode 8) et RSSI (opcode 0x10000102) : optionnels selon le pilote.
        let mut channel: Option<u32> = None;
        let mut d2: *mut c_void = null_mut();
        let mut s2 = 0u32;
        if WlanQueryInterface(handle, guid_ptr, 8, null_mut(), &mut s2, &mut d2, &mut vt) == 0 && !d2.is_null() {
            if s2 as usize >= 4 {
                channel = Some(*(d2 as *const u32));
            }
            WlanFreeMemory(d2);
        }
        let mut rssi: Option<i32> = None;
        let mut d3: *mut c_void = null_mut();
        let mut s3 = 0u32;
        if WlanQueryInterface(handle, guid_ptr, 0x1000_0102, null_mut(), &mut s3, &mut d3, &mut vt) == 0
            && !d3.is_null()
        {
            if s3 as usize >= 4 {
                rssi = Some(*(d3 as *const i32));
            }
            WlanFreeMemory(d3);
        }
        let dlen = info.description.iter().position(|&c| c == 0).unwrap_or(256);
        let adapter = String::from_utf16_lossy(&info.description[..dlen]);
        let band = channel.map(band_from_channel).unwrap_or("?").to_string();

        result = Some(WifiInfo {
            ssid,
            bssid,
            adapter,
            quality,
            rssi_dbm: rssi,
            channel,
            band,
            rx_mbps: rx,
            tx_mbps: tx,
            phy,
        });
        break;
    }
    WlanFreeMemory(list);
    result
}

// ---------------------------------------------------------------------------
// Raccourci clavier global (RegisterHotKey : ne lit pas les touches du jeu)
// ---------------------------------------------------------------------------

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_NOREPEAT: u32 = 0x4000;

const HOTKEY_ID: i32 = 0x4D43;
const WM_HOTKEY: u32 = 0x0312;

/// Enregistre le raccourci pour le thread appelant. Err(code) si deja pris par un autre programme.
pub fn register_hotkey(modifiers: u32, vk: u32) -> Result<(), u32> {
    // SAFETY: enregistrement lie au thread appelant (hwnd = 0).
    unsafe {
        if RegisterHotKey(0, HOTKEY_ID, modifiers | MOD_NOREPEAT, vk) != 0 {
            Ok(())
        } else {
            Err(GetLastError())
        }
    }
}

pub fn unregister_hotkey() {
    // SAFETY: desenregistre l'identifiant du thread appelant.
    unsafe {
        UnregisterHotKey(0, HOTKEY_ID);
    }
}

/// Vide la file de messages du thread et renvoie true si le raccourci a ete presse.
pub fn poll_hotkey() -> bool {
    let mut pressed = false;
    // SAFETY: structure MSG initialisee a zero ; PM_REMOVE = 1.
    unsafe {
        let mut msg: Msg = std::mem::zeroed();
        while PeekMessageW(&mut msg, 0, 0, 0, 1) != 0 {
            if msg.message == WM_HOTKEY && msg.wparam as i32 == HOTKEY_ID {
                pressed = true;
            }
        }
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_windows_abi() {
        // MSG = 48 octets sur x64.
        assert_eq!(std::mem::size_of::<Msg>(), 48);
        // PROCESSENTRY32W = 568 octets sur x64.
        assert_eq!(std::mem::size_of::<ProcessEntry32W>(), 568);
        // WLAN_INTERFACE_INFO = 16 + 512 + 4 = 532 octets.
        assert_eq!(std::mem::size_of::<WlanInterfaceInfo>(), 532);
        // WLAN_ASSOCIATION_ATTRIBUTES = 68 octets.
        assert_eq!(std::mem::size_of::<WlanAssociation>(), 68);
    }

    #[test]
    fn port_and_addr_decoding() {
        // Port 25565 = 0x63DD, octets reseau 63 DD -> DWORD little-endian 0x0000DD63.
        assert_eq!(port_from_raw(0x0000_DD63), 25565);
        // 192.168.1.10 -> octets C0 A8 01 0A -> DWORD LE 0x0A01A8C0.
        assert_eq!(addr_from_raw(0x0A01_A8C0), Ipv4Addr::new(192, 168, 1, 10));
    }

    #[test]
    fn can_list_own_process() {
        // Le processus de test existe : la table de processus doit contenir au moins un exe.
        let pids = find_pids(&["explorer.exe", "svchost.exe", "System"]);
        assert!(!pids.is_empty());
    }

    #[test]
    fn tcp_table_is_readable() {
        let t = tcp_table();
        assert!(t.is_ok(), "tcp_table a échoué : {:?}", t.err());
    }

    #[test]
    fn icmp_loopback_responds() {
        let p = IcmpPinger::new().expect("IcmpCreateFile");
        let rtt = p.ping(Ipv4Addr::new(127, 0, 0, 1), 1000);
        assert!(rtt.is_some());
    }

    #[test]
    fn default_gateway_query_does_not_panic() {
        let _ = default_gateway();
    }

    #[test]
    fn wlan_query_does_not_panic() {
        // Peut renvoyer None (runner sans Wi-Fi) : on verifie seulement l'absence de crash.
        let _ = wlan_current();
    }

    #[test]
    fn if_table_is_readable() {
        let _ = if_octets();
    }

    #[test]
    fn current_process_priority_readable() {
        let pid = std::process::id();
        assert!(get_priority_class(pid).is_some());
    }
}
