use std::{
    collections::BTreeSet,
    ffi::{c_void, CString},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread::{sleep, JoinHandle},
    time::{Duration, Instant},
};

use ipc::proxy::{error, memory_name, socks, state, Request, Setting, Shared, MEMORY_SIZE};
use retour::{Function, GenericDetour};
use windows::{
    core::{s, PCSTR, PSTR},
    Win32::{
        Foundation::{CloseHandle, SetLastError, HANDLE, INVALID_HANDLE_VALUE, WIN32_ERROR},
        Networking::WinSock::{
            getpeername, getsockopt, recv, select, send, shutdown, WSAGetLastError,
            WSASetLastError, FD_SET, SD_BOTH, SEND_RECV_FLAGS, SOCKADDR, SOCKET, SOCKET_ERROR,
            SOCK_STREAM, SOL_SOCKET, SO_ERROR, SO_TYPE, TIMEVAL, WSAEACCES, WSAECONNREFUSED,
            WSAENOTCONN, WSAESHUTDOWN, WSAETIMEDOUT, WSAEWOULDBLOCK, WSA_ERROR,
        },
        System::{
            LibraryLoader::{GetModuleHandleA, GetProcAddress},
            Memory::{
                CreateFileMappingA, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
                MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
            },
            Threading::GetCurrentProcessId,
        },
    },
};

use super::unload::HookCall;

type Connect = unsafe extern "system" fn(SOCKET, *const SOCKADDR, i32) -> i32;
type IcmpSendEcho = unsafe extern "system" fn(
    HANDLE,
    u32,
    *const c_void,
    u16,
    *const c_void,
    *mut c_void,
    u32,
    u32,
) -> u32;
type IcmpSendEcho2 = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *const c_void,
    *const c_void,
    u32,
    *const c_void,
    u16,
    *const c_void,
    *mut c_void,
    u32,
    u32,
) -> u32;
type IcmpSendEcho2Ex = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *const c_void,
    *const c_void,
    u32,
    u32,
    *const c_void,
    u16,
    *const c_void,
    *mut c_void,
    u32,
    u32,
) -> u32;
type Icmp6SendEcho2 = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *const c_void,
    *const c_void,
    *const c_void,
    *const c_void,
    *const c_void,
    u16,
    *const c_void,
    *mut c_void,
    u32,
    u32,
) -> u32;
type QuerySystemInformation = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;

const AF_INET: u16 = 2;
const AF_INET6: u16 = 23;
const SOCKADDR_IN_LEN: i32 = 16;
const SOCKADDR_IN6_LEN: i32 = 28;
const IP_REQ_TIMED_OUT: u32 = 11010;
const SYSTEM_EXTENDED_HANDLE_INFORMATION: u32 = 64;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
const HANDLE_TABLE_LIMIT: usize = 256 << 20;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const PENDING_WAIT: Duration = Duration::from_secs(5);
const PENDING_KEPT: usize = 32;
const ROUTED_KEPT: usize = 256;
const WATCH_EVERY: Duration = Duration::from_millis(200);

static CONNECT: OnceLock<GenericDetour<Connect>> = OnceLock::new();
static ICMP_SEND_ECHO: OnceLock<GenericDetour<IcmpSendEcho>> = OnceLock::new();
static ICMP_SEND_ECHO2: OnceLock<GenericDetour<IcmpSendEcho2>> = OnceLock::new();
static ICMP_SEND_ECHO2_EX: OnceLock<GenericDetour<IcmpSendEcho2Ex>> = OnceLock::new();
static ICMP6_SEND_ECHO2: OnceLock<GenericDetour<Icmp6SendEcho2>> = OnceLock::new();
static MAPPING: OnceLock<Mapping> = OnceLock::new();
static RESOLVED: Mutex<Option<(u32, Vec<SocketAddr>)>> = Mutex::new(None);
static PENDING: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static ROUTED: Mutex<Vec<(usize, SocketAddr)>> = Mutex::new(Vec::new());
static STOPPED: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

struct Mapping {
    handle: isize,
    view: usize,
    shared: Shared,
}

#[repr(C)]
struct HandleEntry {
    object: usize,
    process: usize,
    handle: usize,
    access: u32,
    backtrace: u16,
    type_index: u16,
    attributes: u32,
    reserved: u32,
}

#[derive(Clone, Copy)]
enum Ready {
    Read,
    Write,
}

unsafe fn open_mapping(pid: u32) -> Option<Mapping> {
    let name = CString::new(memory_name(pid)).ok()?;
    let handle = CreateFileMappingA(
        INVALID_HANDLE_VALUE,
        None,
        PAGE_READWRITE,
        0,
        MEMORY_SIZE as u32,
        PCSTR(name.as_ptr().cast()),
    )
    .ok()?;
    let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, MEMORY_SIZE);
    let Some(ptr) = NonNull::new(view.Value.cast::<u8>()) else {
        let _ = CloseHandle(handle);
        return None;
    };
    Some(Mapping {
        handle: handle.0 as isize,
        view: ptr.as_ptr() as usize,
        shared: Shared::attach(ptr),
    })
}

fn setting() -> (u32, Setting) {
    match MAPPING.get() {
        Some(mapping) => mapping.shared.read(),
        None => (0, Setting::Unreadable),
    }
}

fn current_sequence() -> u32 {
    setting().0
}

fn status() -> Option<&'static ipc::proxy::Status> {
    MAPPING.get().map(|mapping| mapping.shared.status())
}

fn fail(code: i32) -> i32 {
    if let Some(status) = status() {
        status.fail(code);
    }
    let winsock = match code {
        error::TIMEOUT => WSAETIMEDOUT,
        error::DATAGRAM | error::UNREADABLE => WSAEACCES,
        _ => WSAECONNREFUSED,
    };
    unsafe { refuse(winsock) }
}

unsafe fn refuse(code: WSA_ERROR) -> i32 {
    WSASetLastError(code.0);
    SOCKET_ERROR
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

unsafe fn read_sockaddr(name: *const SOCKADDR, len: i32) -> Option<SocketAddr> {
    if name.is_null() || len < 2 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(name.cast::<u8>(), len as usize);
    let port = |b: &[u8]| u16::from_be_bytes([b[2], b[3]]);
    match u16::from_ne_bytes([bytes[0], bytes[1]]) {
        AF_INET if len >= SOCKADDR_IN_LEN => {
            let ip = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
            Some(SocketAddr::V4(SocketAddrV4::new(ip, port(bytes))))
        }
        AF_INET6 if len >= SOCKADDR_IN6_LEN => {
            let flow = u32::from_ne_bytes(bytes[4..8].try_into().ok()?);
            let ip = Ipv6Addr::from(<[u8; 16]>::try_from(&bytes[8..24]).ok()?);
            let scope = u32::from_ne_bytes(bytes[24..28].try_into().ok()?);
            Some(SocketAddr::V6(SocketAddrV6::new(
                ip,
                port(bytes),
                flow,
                scope,
            )))
        }
        _ => None,
    }
}

fn write_sockaddr(address: &SocketAddr) -> ([u8; SOCKADDR_IN6_LEN as usize], i32) {
    let mut bytes = [0u8; SOCKADDR_IN6_LEN as usize];
    bytes[2..4].copy_from_slice(&address.port().to_be_bytes());
    match address {
        SocketAddr::V4(v4) => {
            bytes[0..2].copy_from_slice(&AF_INET.to_ne_bytes());
            bytes[4..8].copy_from_slice(&v4.ip().octets());
            (bytes, SOCKADDR_IN_LEN)
        }
        SocketAddr::V6(v6) => {
            bytes[0..2].copy_from_slice(&AF_INET6.to_ne_bytes());
            bytes[4..8].copy_from_slice(&v6.flowinfo().to_ne_bytes());
            bytes[8..24].copy_from_slice(&v6.ip().octets());
            bytes[24..28].copy_from_slice(&v6.scope_id().to_ne_bytes());
            (bytes, SOCKADDR_IN6_LEN)
        }
    }
}

fn socks_address(target: &SocketAddr) -> socks::Address {
    match target.ip() {
        IpAddr::V4(ip) => socks::Address::V4(ip.octets()),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(v4) => socks::Address::V4(v4.octets()),
            None => socks::Address::V6(ip.octets()),
        },
    }
}

fn proxy_addresses(sequence: u32, request: &Request) -> Result<Vec<SocketAddr>, i32> {
    let mut resolved = RESOLVED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, addresses)) = resolved.as_ref() {
        if *at == sequence {
            return Ok(addresses.clone());
        }
    }
    let host = request.host.text();
    let addresses: Vec<SocketAddr> = (host.as_str(), request.port)
        .to_socket_addrs()
        .map_err(|_| error::RESOLVE)?
        .collect();
    if addresses.is_empty() {
        return Err(error::RESOLVE);
    }
    *resolved = Some((sequence, addresses.clone()));
    Ok(addresses)
}

unsafe fn original(s: SOCKET, name: *const SOCKADDR, len: i32) -> i32 {
    match CONNECT.get() {
        Some(detour) => detour.call(s, name, len),
        None => refuse(WSAECONNREFUSED),
    }
}

unsafe fn socket_option(s: SOCKET, option: i32) -> Option<i32> {
    let mut value = 0i32;
    let mut len = size_of::<i32>() as i32;
    let ok = getsockopt(
        s,
        SOL_SOCKET,
        option,
        PSTR((&mut value as *mut i32).cast()),
        &mut len,
    ) == 0;
    ok.then_some(value)
}

unsafe fn wait(s: SOCKET, ready: Ready, deadline: Instant) -> Result<(), i32> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(error::TIMEOUT);
    }
    let mut wanted = FD_SET {
        fd_count: 1,
        fd_array: [SOCKET(0); 64],
    };
    wanted.fd_array[0] = s;
    let mut failed = wanted;
    let timeout = TIMEVAL {
        tv_sec: left.as_secs() as i32,
        tv_usec: left.subsec_micros() as i32,
    };
    let (read, write) = match ready {
        Ready::Read => (Some(&mut wanted as *mut FD_SET), None),
        Ready::Write => (None, Some(&mut wanted as *mut FD_SET)),
    };
    match select(0, read, write, Some(&mut failed), Some(&timeout)) {
        0 => Err(error::TIMEOUT),
        n if n < 0 => Err(error::CONNECT),
        _ if failed.fd_count > 0 => Err(error::CONNECT),
        _ => Ok(()),
    }
}

unsafe fn send_all(s: SOCKET, mut bytes: &[u8], deadline: Instant) -> Result<(), i32> {
    while !bytes.is_empty() {
        let sent = send(s, bytes, SEND_RECV_FLAGS(0));
        if sent > 0 {
            bytes = &bytes[sent as usize..];
        } else if sent == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK {
            wait(s, Ready::Write, deadline)?;
        } else {
            return Err(error::CONNECT);
        }
    }
    Ok(())
}

unsafe fn recv_exact(s: SOCKET, buffer: &mut [u8], deadline: Instant) -> Result<(), i32> {
    let mut filled = 0;
    while filled < buffer.len() {
        wait(s, Ready::Read, deadline)?;
        let got = recv(s, &mut buffer[filled..], SEND_RECV_FLAGS(0));
        if got > 0 {
            filled += got as usize;
        } else if got == 0 {
            return Err(error::REPLY);
        } else if WSAGetLastError() != WSAEWOULDBLOCK {
            return Err(error::CONNECT);
        }
    }
    Ok(())
}

unsafe fn connect_to_proxy(s: SOCKET, proxy: &SocketAddr, deadline: Instant) -> Result<(), i32> {
    let (address, len) = write_sockaddr(proxy);
    if original(s, address.as_ptr().cast(), len) == 0 {
        return Ok(());
    }
    if WSAGetLastError() != WSAEWOULDBLOCK {
        return Err(error::CONNECT);
    }
    wait(s, Ready::Write, deadline)?;
    match socket_option(s, SO_ERROR) {
        Some(0) => Ok(()),
        _ => Err(error::CONNECT),
    }
}

unsafe fn handshake(
    s: SOCKET,
    target: &SocketAddr,
    request: &Request,
    deadline: Instant,
) -> Result<(), i32> {
    let login = request.logs_in();
    let mut reply = [0u8; 2];
    send_all(s, &socks::greeting(login), deadline)?;
    recv_exact(s, &mut reply, deadline)?;
    if socks::method(reply, login)? {
        let message = socks::login(request.username.bytes(), request.password.bytes());
        send_all(s, &message, deadline)?;
        recv_exact(s, &mut reply, deadline)?;
        socks::logged_in(reply)?;
    }
    send_all(
        s,
        &socks::connect(socks_address(target), target.port()),
        deadline,
    )?;
    let mut head = [0u8; socks::REPLY_HEAD];
    recv_exact(s, &mut head, deadline)?;
    let mut rest = vec![0u8; socks::reply_rest(head)?];
    recv_exact(s, &mut rest, deadline)
}

unsafe fn through_proxy(
    s: SOCKET,
    target: &SocketAddr,
    sequence: u32,
    request: &Request,
) -> Result<SocketAddr, i32> {
    if socket_option(s, SO_TYPE) != Some(SOCK_STREAM.0) {
        return Err(error::DATAGRAM);
    }
    let addresses = proxy_addresses(sequence, request)?;
    let proxy = *addresses
        .iter()
        .find(|address| address.is_ipv4() == target.is_ipv4())
        .ok_or(error::FAMILY)?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    connect_to_proxy(s, &proxy, deadline)?;
    handshake(s, target, request, deadline)?;
    if current_sequence() != sequence {
        return Err(error::CHANGED);
    }
    Ok(proxy)
}

fn remember_routed(s: SOCKET, proxy: SocketAddr) {
    let mut routed = ROUTED.lock().unwrap_or_else(|e| e.into_inner());
    routed.retain(|&(handle, _)| handle != s.0);
    if routed.len() >= ROUTED_KEPT {
        routed.remove(0);
    }
    routed.push((s.0, proxy));
}

fn remember_pending(s: SOCKET) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if pending.len() >= PENDING_KEPT {
        pending.remove(0);
    }
    pending.push(s.0);
}

unsafe fn direct(s: SOCKET, name: *const SOCKADDR, len: i32, sequence: u32) -> i32 {
    let result = original(s, name, len);
    let error = WSAGetLastError();
    if result == 0 || error == WSAEWOULDBLOCK {
        remember_pending(s);
    }
    if current_sequence() != sequence {
        let _ = shutdown(s, SD_BOTH);
        return fail(error::CHANGED);
    }
    if result != 0 {
        WSASetLastError(error.0);
    }
    result
}

unsafe fn route(s: SOCKET, name: *const SOCKADDR, len: i32) -> i32 {
    let Some(target) = read_sockaddr(name, len) else {
        return original(s, name, len);
    };
    if is_loopback(target.ip()) {
        return original(s, name, len);
    }
    match setting() {
        (sequence, Setting::Off) => direct(s, name, len, sequence),
        (_, Setting::Unreadable) => fail(error::UNREADABLE),
        (sequence, Setting::On(request)) => match through_proxy(s, &target, sequence, &request) {
            Ok(proxy) => {
                remember_routed(s, proxy);
                if let Some(status) = status() {
                    status.proxied.fetch_add(1, Ordering::Relaxed);
                }
                0
            }
            Err(code) => {
                let _ = shutdown(s, SD_BOTH);
                fail(code)
            }
        },
    }
}

unsafe extern "system" fn hooked_connect(s: SOCKET, name: *const SOCKADDR, len: i32) -> i32 {
    let _call = HookCall::enter();
    catch_unwind(AssertUnwindSafe(|| route(s, name, len))).unwrap_or_else(|_| fail(error::CONNECT))
}

fn pings_blocked() -> bool {
    !matches!(setting().1, Setting::Off)
}

fn blocked_ping() -> u32 {
    unsafe { SetLastError(WIN32_ERROR(IP_REQ_TIMED_OUT)) };
    0
}

unsafe extern "system" fn hooked_icmp_send_echo(
    icmp: HANDLE,
    destination: u32,
    data: *const c_void,
    size: u16,
    options: *const c_void,
    reply: *mut c_void,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let _call = HookCall::enter();
    match ICMP_SEND_ECHO.get() {
        Some(detour) if !pings_blocked() => detour.call(
            icmp,
            destination,
            data,
            size,
            options,
            reply,
            reply_size,
            timeout,
        ),
        _ => blocked_ping(),
    }
}

unsafe extern "system" fn hooked_icmp_send_echo2(
    icmp: HANDLE,
    event: HANDLE,
    apc: *const c_void,
    context: *const c_void,
    destination: u32,
    data: *const c_void,
    size: u16,
    options: *const c_void,
    reply: *mut c_void,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let _call = HookCall::enter();
    match ICMP_SEND_ECHO2.get() {
        Some(detour) if !pings_blocked() => detour.call(
            icmp,
            event,
            apc,
            context,
            destination,
            data,
            size,
            options,
            reply,
            reply_size,
            timeout,
        ),
        _ => blocked_ping(),
    }
}

unsafe extern "system" fn hooked_icmp_send_echo2_ex(
    icmp: HANDLE,
    event: HANDLE,
    apc: *const c_void,
    context: *const c_void,
    source: u32,
    destination: u32,
    data: *const c_void,
    size: u16,
    options: *const c_void,
    reply: *mut c_void,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let _call = HookCall::enter();
    match ICMP_SEND_ECHO2_EX.get() {
        Some(detour) if !pings_blocked() => detour.call(
            icmp,
            event,
            apc,
            context,
            source,
            destination,
            data,
            size,
            options,
            reply,
            reply_size,
            timeout,
        ),
        _ => blocked_ping(),
    }
}

unsafe extern "system" fn hooked_icmp6_send_echo2(
    icmp: HANDLE,
    event: HANDLE,
    apc: *const c_void,
    context: *const c_void,
    source: *const c_void,
    destination: *const c_void,
    data: *const c_void,
    size: u16,
    options: *const c_void,
    reply: *mut c_void,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let _call = HookCall::enter();
    match ICMP6_SEND_ECHO2.get() {
        Some(detour) if !pings_blocked() => detour.call(
            icmp,
            event,
            apc,
            context,
            source,
            destination,
            data,
            size,
            options,
            reply,
            reply_size,
            timeout,
        ),
        _ => blocked_ping(),
    }
}

unsafe fn install<T: Function>(
    module: PCSTR,
    name: PCSTR,
    detour: T,
    slot: &OnceLock<GenericDetour<T>>,
) -> bool {
    let what = name.to_string().unwrap_or_default();
    let Some(address) = GetModuleHandleA(module)
        .ok()
        .and_then(|module| GetProcAddress(module, name))
    else {
        println!("[Agent]: Proxy: {what} not found.\r\n");
        return false;
    };
    let hooked = GenericDetour::new(T::from_ptr(address as usize as *const ()), detour)
        .and_then(|hook| hook.enable().map(|()| hook));
    match hooked {
        Ok(hook) => slot.set(hook).is_ok(),
        Err(why) => {
            println!("[Agent]: Proxy: couldn't hook {what}: {why}.\r\n");
            false
        }
    }
}

unsafe fn peer_of(handle: usize) -> Result<SocketAddr, WSA_ERROR> {
    let mut address = [0u8; SOCKADDR_IN6_LEN as usize];
    let mut len = SOCKADDR_IN6_LEN;
    if getpeername(SOCKET(handle), address.as_mut_ptr().cast(), &mut len) != 0 {
        return Err(WSAGetLastError());
    }
    read_sockaddr(address.as_ptr().cast(), len).ok_or(WSAENOTCONN)
}

unsafe fn own_handles() -> Vec<usize> {
    let Some(query) = GetModuleHandleA(s!("ntdll.dll"))
        .ok()
        .and_then(|ntdll| GetProcAddress(ntdll, s!("NtQuerySystemInformation")))
    else {
        return Vec::new();
    };
    let query: QuerySystemInformation = std::mem::transmute(query);
    let pid = GetCurrentProcessId() as usize;
    let mut size = 1usize << 20;
    loop {
        let mut buffer = vec![0u64; size / 8];
        let mut needed = 0u32;
        let status = query(
            SYSTEM_EXTENDED_HANDLE_INFORMATION,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            &mut needed,
        );
        if status == STATUS_INFO_LENGTH_MISMATCH && size < HANDLE_TABLE_LIMIT {
            size = (needed as usize + (1 << 16)).max(size * 2);
            continue;
        }
        if status < 0 {
            println!("[Agent]: Proxy: couldn't list the client's handles ({status:#x}).\r\n");
            return Vec::new();
        }
        let room = (buffer.len() * 8 - 16) / size_of::<HandleEntry>();
        let count = (buffer[0] as usize).min(room);
        let entries =
            std::slice::from_raw_parts(buffer.as_ptr().add(2).cast::<HandleEntry>(), count);
        return entries
            .iter()
            .filter(|entry| entry.process == pid)
            .map(|entry| entry.handle)
            .collect();
    }
}

unsafe fn already_shut(handle: usize) -> bool {
    send(SOCKET(handle), &[], SEND_RECV_FLAGS(0)) == SOCKET_ERROR
        && WSAGetLastError() == WSAESHUTDOWN
}

unsafe fn shut(handle: usize) -> u32 {
    if already_shut(handle) {
        return 0;
    }
    let _ = shutdown(SOCKET(handle), SD_BOTH);
    1
}

unsafe fn wait_for_pending(mut waiting: Vec<usize>, drops: &dyn Fn(SocketAddr) -> bool) -> u32 {
    let deadline = Instant::now() + PENDING_WAIT;
    let mut dropped = 0;
    while !waiting.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut writable = FD_SET {
            fd_count: waiting.len() as u32,
            fd_array: [SOCKET(0); 64],
        };
        for (slot, &handle) in writable.fd_array.iter_mut().zip(&waiting) {
            *slot = SOCKET(handle);
        }
        let mut failed = writable;
        let timeout = TIMEVAL {
            tv_sec: left.as_secs() as i32,
            tv_usec: left.subsec_micros() as i32,
        };
        if select(
            0,
            None,
            Some(&mut writable),
            Some(&mut failed),
            Some(&timeout),
        ) <= 0
        {
            break;
        }
        let done: Vec<usize> = writable.fd_array[..writable.fd_count as usize]
            .iter()
            .chain(&failed.fd_array[..failed.fd_count as usize])
            .map(|s| s.0)
            .collect();
        for &handle in &done {
            if peer_of(handle).is_ok_and(drops) {
                dropped += shut(handle);
            }
        }
        waiting.retain(|handle| !done.contains(handle));
    }
    dropped
}

unsafe fn sweep(keep: Option<&[SocketAddr]>) -> u32 {
    let outside = |peer: SocketAddr| {
        keep.is_some_and(|keep| !is_loopback(peer.ip()) && !keep.contains(&peer))
    };
    let kept = |proxy: &SocketAddr| keep.is_some_and(|keep| keep.contains(proxy));
    let pending = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    let stale: Vec<(usize, SocketAddr)> = {
        let mut routed = ROUTED.lock().unwrap_or_else(|e| e.into_inner());
        let stale = routed.iter().copied().filter(|(_, proxy)| !kept(proxy)).collect();
        routed.retain(|(_, proxy)| kept(proxy));
        stale
    };
    let handles: BTreeSet<usize> = own_handles()
        .into_iter()
        .chain(pending.iter().copied())
        .chain(stale.iter().map(|&(handle, _)| handle))
        .collect();
    let mut dropped = 0;
    let mut waiting = Vec::new();
    for handle in handles {
        match peer_of(handle) {
            Ok(peer) if outside(peer) || stale.contains(&(handle, peer)) => {
                dropped += shut(handle);
            }
            Err(code) if code == WSAENOTCONN && pending.contains(&handle) => waiting.push(handle),
            _ => {}
        }
    }
    dropped + wait_for_pending(waiting, &outside)
}

fn adopt(sequence: u32, setting: Setting, hooked: bool) {
    let (state, keep) = match setting {
        Setting::Off => (state::OFF, None),
        Setting::On(request) => (
            state::ON,
            Some(proxy_addresses(sequence, &request).unwrap_or_default()),
        ),
        Setting::Unreadable => (state::UNREADABLE, Some(Vec::new())),
    };
    let dropped = unsafe { sweep(keep.as_deref()) };
    if dropped > 0 {
        println!("[Agent]: Proxy: shut down {dropped} connections that didn't go through it.\r\n");
    }
    if let Some(status) = status() {
        status.dropped.fetch_add(dropped, Ordering::Relaxed);
        status.adopt(sequence, if hooked { state } else { state::UNHOOKED });
    }
}

fn watch(hooked: bool) {
    let mut adopted = None;
    while !STOPPED.load(Ordering::SeqCst) {
        let (sequence, setting) = setting();
        if adopted != Some(sequence) {
            let result = catch_unwind(AssertUnwindSafe(|| adopt(sequence, setting, hooked)));
            if result.is_err() {
                println!("[Agent]: Proxy: applying the setting failed.\r\n");
            }
            adopted = Some(sequence);
        }
        sleep(WATCH_EVERY);
    }
}

pub unsafe fn start(pid: u32) {
    match open_mapping(pid) {
        Some(mapping) => drop(MAPPING.set(mapping)),
        None => println!(
            "[Agent]: Proxy: couldn't open the proxy setting, so every outside connection is blocked.\r\n"
        ),
    }
    let hooked = install(
        s!("ws2_32.dll"),
        s!("connect"),
        hooked_connect as Connect,
        &CONNECT,
    );
    let iphlpapi = s!("iphlpapi.dll");
    install(
        iphlpapi,
        s!("IcmpSendEcho"),
        hooked_icmp_send_echo as IcmpSendEcho,
        &ICMP_SEND_ECHO,
    );
    install(
        iphlpapi,
        s!("IcmpSendEcho2"),
        hooked_icmp_send_echo2 as IcmpSendEcho2,
        &ICMP_SEND_ECHO2,
    );
    install(
        iphlpapi,
        s!("IcmpSendEcho2Ex"),
        hooked_icmp_send_echo2_ex as IcmpSendEcho2Ex,
        &ICMP_SEND_ECHO2_EX,
    );
    install(
        iphlpapi,
        s!("Icmp6SendEcho2"),
        hooked_icmp6_send_echo2 as Icmp6SendEcho2,
        &ICMP6_SEND_ECHO2,
    );
    STOPPED.store(false, Ordering::SeqCst);
    match std::thread::Builder::new()
        .name("proxy".into())
        .spawn(move || watch(hooked))
    {
        Ok(thread) => *THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread),
        Err(why) => println!("[Agent]: Proxy: couldn't start its watcher: {why}.\r\n"),
    }
}

pub fn stop() {
    STOPPED.store(true, Ordering::SeqCst);
    if let Some(thread) = THREAD.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = thread.join();
    }
}

fn disable<T: Function>(slot: &OnceLock<GenericDetour<T>>) {
    if let Some(detour) = slot.get().filter(|detour| detour.is_enabled()) {
        if let Err(why) = unsafe { detour.disable() } {
            println!("[Agent]: Proxy: couldn't unhook: {why}.\r\n");
        }
    }
}

pub fn unhook() {
    disable(&CONNECT);
    disable(&ICMP_SEND_ECHO);
    disable(&ICMP_SEND_ECHO2);
    disable(&ICMP_SEND_ECHO2_EX);
    disable(&ICMP6_SEND_ECHO2);
}

pub unsafe fn close() {
    if let Some(mapping) = MAPPING.get() {
        let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
            Value: mapping.view as *mut c_void,
        });
        let _ = CloseHandle(HANDLE(mapping.handle as *mut c_void));
    }
}
