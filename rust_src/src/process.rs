//! Functions operating on process.
use libc;
use std::{mem, ptr};

use remacs_macros::lisp_fn;

use crate::{
    buffers::{current_buffer, get_buffer, LispBufferOrName, LispBufferRef},
    eval::run_hook_with_args_until_success,
    lisp::{ExternalPtr, LispObject, ProcessIter},
    lists::{assoc, car, cdr, plist_put},
    multibyte::LispStringRef,
    remacs_sys::{
        add_process_read_fd, block_child_signal, current_thread, delete_read_fd,
        emacs_get_tty_pgrp, list_system_processes, pset_status, redisplay_preserve_echo_area,
        send_process, setup_process_coding_systems, sigset_t, status_notify, tcflush,
        unblock_child_signal, update_process_tick, update_status, Fdelete_process, Fmapcar,
        CDISABLE, PROCESSP, STRING_BYTES,
    },
    remacs_sys::{pid_t, pvec_type, EmacsInt, Lisp_Process, Lisp_Type, Vprocess_alist},
    remacs_sys::{
        QCbuffer, QCfilter, QCsentinel, Qcdr, Qclosed, Qexit, Qinternal_default_process_filter,
        Qinternal_default_process_sentinel, Qinterrupt_process_functions, Qlambda, Qlisten, Qlistp,
        Qnetwork, Qnil, Qopen, Qpipe, Qprocessp, Qreal, Qrun, Qserial, Qstop, Qt,
    },
};

pub type LispProcessRef = ExternalPtr<Lisp_Process>;

impl LispProcessRef {
    fn ptype(self) -> LispObject {
        self.type_
    }

    fn set_plist(&mut self, plist: LispObject) {
        self.plist = plist;
    }

    fn set_buffer(&mut self, buffer: Option<LispBufferRef>) {
        self.buffer = buffer.into();
    }

    fn set_childp(&mut self, childp: LispObject) {
        self.childp = childp;
    }
}

impl LispObject {
    pub fn is_process(self) -> bool {
        self.as_vectorlike()
            .map_or(false, |v| v.is_pseudovector(pvec_type::PVEC_PROCESS))
    }

    pub fn as_process(self) -> Option<LispProcessRef> {
        self.into()
    }
}

impl From<LispObject> for LispProcessRef {
    fn from(o: LispObject) -> Self {
        o.as_process().unwrap_or_else(|| wrong_type!(Qprocessp, o))
    }
}

impl From<LispProcessRef> for LispObject {
    fn from(p: LispProcessRef) -> Self {
        LispObject::tag_ptr(p, Lisp_Type::Lisp_Vectorlike)
    }
}

impl From<LispObject> for Option<LispProcessRef> {
    fn from(o: LispObject) -> Self {
        o.as_vectorlike().and_then(|v| v.as_process())
    }
}

macro_rules! for_each_process {
    ($name:ident => $action:block) => {
        for $name in ProcessIter::new()
            $action
    };
}

/// This is how commands for the user decode process arguments.  It
/// accepts a process, a process name, a buffer, a buffer name, or nil.
/// Buffers denote the first process in the buffer, and nil denotes the
/// current buffer.
#[no_mangle]
pub extern "C" fn get_process(name: LispObject) -> LispObject {
    let proc_or_buf = if name.is_string() {
        let mut obj = get_process_lisp(name);

        if obj.is_nil() {
            obj = get_buffer(LispBufferOrName::from(name)).map_or_else(
                || error!("Process {} does not exist", name),
                LispObject::from,
            );
        }
        obj
    } else if name.is_nil() {
        current_buffer()
    } else {
        name
    };

    // Now obj should be either a buffer object or a process object.
    match proc_or_buf.as_buffer() {
        Some(b) => {
            let name = b
                .name()
                .as_string()
                .unwrap_or_else(|| error!("Attempt to get process for a dead buffer"));
            match get_buffer_process_internal(Some(b)) {
                None => error!("Buffer {:?} has no process.", unsafe {
                    name.u.s.data as *mut libc::c_char
                }),
                Some(proc) => proc.into(),
            }
        }
        None => LispProcessRef::from(proc_or_buf).into(),
    }
}

/// Return t if OBJECT is a process.
#[lisp_fn]
pub fn processp(object: LispObject) -> bool {
    object.is_process()
}

/// Return the process named NAME, or nil if there is none.
#[lisp_fn(name = "get-process", c_name = "get_process")]
pub fn get_process_lisp(name: LispObject) -> LispObject {
    if name.is_process() {
        name
    } else {
        LispStringRef::from(name);
        cdr(assoc(name, unsafe { Vprocess_alist }, Qnil))
    }
}

/// Return the name of PROCESS, as a string.
/// This is the name of the program invoked in PROCESS,
/// possibly modified to make it unique among process names.
#[lisp_fn]
pub fn process_name(process: LispProcessRef) -> LispObject {
    process.name
}

/// Return the buffer PROCESS is associated with.
/// The default process filter inserts output from PROCESS into this buffer.
#[lisp_fn]
pub fn process_buffer(process: LispProcessRef) -> LispObject {
    process.buffer
}

/// Return the process id of PROCESS.
/// This is the pid of the external process which PROCESS uses or talks to.
/// For a network, serial, and pipe connections, this value is nil.
#[lisp_fn]
pub fn process_id(process: LispProcessRef) -> Option<EmacsInt> {
    if process.pid == 0 {
        None
    } else {
        Some(EmacsInt::from(process.pid))
    }
}

/// Return the (or a) live process associated with BUFFER.
/// BUFFER may be a buffer or the name of one.
/// Return nil if all processes associated with BUFFER have been
/// deleted or killed.
#[lisp_fn]
pub fn get_buffer_process(buffer_or_name: Option<LispBufferOrName>) -> Option<LispProcessRef> {
    get_buffer_process_internal(buffer_or_name.and_then(|b| b.into()))
}

pub fn get_buffer_process_internal(buffer: Option<LispBufferRef>) -> Option<LispProcessRef> {
    if let Some(buf) = buffer {
        let obj = LispObject::from(buf);
        for_each_process!(p => {
            if obj.eq(p.buffer) {
                return Some(p);
            }
        });
    }
    None
}

/// Return the name of the terminal PROCESS uses, or nil if none.
/// This is the terminal that the process itself reads and writes on,
/// not the name of the pty that Emacs uses to talk with that terminal.
#[lisp_fn]
pub fn process_tty_name(process: LispProcessRef) -> LispObject {
    process.tty_name
}

/// Return the command that was executed to start PROCESS.  This is a
/// list of strings, the first string being the program executed and
/// the rest of the strings being the arguments given to it.  For a
/// network or serial or pipe connection, this is nil (process is
/// running) or t (process is stopped).
#[lisp_fn]
pub fn process_command(process: LispProcessRef) -> LispObject {
    process.command
}

/// Return the filter function of PROCESS.
/// See `set-process-filter' for more info on filter functions.
#[lisp_fn]
pub fn process_filter(process: LispProcessRef) -> LispObject {
    process.filter
}

/// Return the sentinel of PROCESS.
/// See `set-process-sentinel' for more info on sentinels.
#[lisp_fn]
pub fn process_sentinel(process: LispProcessRef) -> LispObject {
    process.sentinel
}

/// Return the marker for the end of the last output from PROCESS.
#[lisp_fn]
pub fn process_mark(process: LispProcessRef) -> LispObject {
    process.mark
}

/// Return a list of all processes that are Emacs sub-processes.
#[lisp_fn]
pub fn process_list() -> LispObject {
    unsafe { Fmapcar(Qcdr, Vprocess_alist) }
}

/// Return the plist of PROCESS.
#[lisp_fn]
pub fn process_plist(process: LispProcessRef) -> LispObject {
    process.plist
}

/// Replace the plist of PROCESS with PLIST.  Return PLIST.
#[lisp_fn]
pub fn set_process_plist(mut process: LispProcessRef, plist: LispObject) -> LispObject {
    if plist.is_list() {
        process.set_plist(plist);
        plist
    } else {
        wrong_type!(Qlistp, plist)
    }
}

/// Return the status of PROCESS.
/// The returned value is one of the following symbols:
/// run  -- for a process that is running.
/// stop -- for a process stopped but continuable.
/// exit -- for a process that has exited.
/// signal -- for a process that has got a fatal signal.
/// open -- for a network stream connection that is open.
/// listen -- for a network stream server that is listening.
/// closed -- for a network stream connection that is closed.
/// connect -- when waiting for a non-blocking connection to complete.
/// failed -- when a non-blocking connection has failed.
/// nil -- if arg is a process name and no such process exists.
/// PROCESS may be a process, a buffer, the name of a process, or
/// nil, indicating the current buffer's process.
#[lisp_fn]
pub fn process_status(process: LispObject) -> LispObject {
    let mut process = match get_process(process) {
        Qnil => return process,
        p => LispProcessRef::from(p),
    };

    if process.raw_status_new() {
        unsafe { update_status(process.as_mut()) };
    }
    let mut status = process.status;
    if let Some((a, _)) = status.into() {
        status = a;
    };
    let process_type = process.ptype();
    if process_type.eq(Qnetwork) || process_type.eq(Qserial) || process_type.eq(Qpipe) {
        let process_command = process.command;
        if status.eq(Qexit) {
            status = Qclosed;
        } else if process_command.eq(Qt) {
            status = Qstop;
        } else if status.eq(Qrun) {
            status = Qopen;
        }
    }
    status
}

/// Return a cons of coding systems for decoding and encoding of PROCESS.
#[lisp_fn]
pub fn process_coding_system(process: LispProcessRef) -> (LispObject, LispObject) {
    (process.decode_coding_system, process.encode_coding_system)
}

/// Return the locking thread of PROCESS.
/// If PROCESS is unlocked, this function returns nil.
#[lisp_fn]
pub fn process_thread(process: LispProcessRef) -> LispObject {
    process.thread
}

/// Return the connection type of PROCESS.
/// The value is either the symbol `real', `network', `serial', or `pipe'.
/// PROCESS may be a process, a buffer, the name of a process or buffer, or
/// nil, indicating the current buffer's process.
#[lisp_fn]
pub fn process_type(process: LispObject) -> LispObject {
    let p_ref: LispProcessRef = get_process(process).into();
    p_ref.ptype()
}

/// Set buffer associated with PROCESS to BUFFER (a buffer, or nil).
/// Return BUFFER.
#[lisp_fn]
pub fn set_process_buffer(
    mut process: LispProcessRef,
    buffer: Option<LispBufferRef>,
) -> LispObject {
    process.set_buffer(buffer);
    let process_type = process.ptype();
    let netconn1_p = process_type.eq(Qnetwork);
    let serialconn1_p = process_type.eq(Qserial);
    let pipeconn1_p = process_type.eq(Qpipe);

    if netconn1_p || serialconn1_p || pipeconn1_p {
        let childp = process.childp;
        process.set_childp(plist_put(childp, QCbuffer, buffer.into()));
    }
    unsafe { setup_process_coding_systems(process.into()) };
    buffer.into()
}

/// Give PROCESS the filter function FILTER; nil means default.
/// A value of t means stop accepting output from the process.
///
/// When a process has a non-default filter, its buffer is not used for output.
/// Instead, each time it does output, the entire string of output is
/// passed to the filter.
///
/// The filter gets two arguments: the process and the string of output.
/// The string argument is normally a multibyte string, except:
/// - if the process's input coding system is no-conversion or raw-text,
///   it is a unibyte string (the non-converted input).
#[lisp_fn]
pub fn set_process_filter(mut process: LispProcessRef, mut filter: LispObject) -> LispObject {
    // Don't signal an error if the process's input file descriptor
    // is closed.  This could make debugging Lisp more difficult,
    // for example when doing something like
    //
    // (setq process (start-process ...))
    // (debug)
    // (set-process-filter process ...)
    filter = pset_filter(process, filter);
    set_process_filter_masks(process);

    let process_type = process.ptype();
    let netconn1_p = process_type.eq(Qnetwork);
    let serialconn1_p = process_type.eq(Qserial);
    let pipeconn1_p = process_type.eq(Qpipe);

    if netconn1_p || serialconn1_p || pipeconn1_p {
        let childp = process.childp;
        process.set_childp(plist_put(childp, QCfilter, filter));
    }
    unsafe { setup_process_coding_systems(process.into()) };
    filter
}

fn pset_filter(mut process: LispProcessRef, val: LispObject) -> LispObject {
    let filter = if val.is_nil() {
        Qinternal_default_process_filter
    } else {
        val
    };
    process.filter = filter;
    filter
}

fn set_process_filter_masks(process: LispProcessRef) {
    if process.infd != -1 && process.filter.eq(Qt) {
        if !process.status.eq(Qlisten) {
            unsafe { delete_read_fd(process.infd) };
        // Network or serial process not stopped:
        } else if process.command.eq(Qt) {
            unsafe { add_process_read_fd(process.infd) };
        }
    }
}

/// Give PROCESS the sentinel SENTINEL; nil for default.
/// The sentinel is called as a function when the process changes state.
/// It gets two arguments: the process, and a string describing the change.
#[lisp_fn]
pub fn set_process_sentinel(mut process: LispProcessRef, mut sentinel: LispObject) -> LispObject {
    sentinel = pset_sentinel(process, sentinel);
    let process_type = process.ptype();
    let netconn1_p = process_type.eq(Qnetwork);
    let serialconn1_p = process_type.eq(Qserial);
    let pipeconn1_p = process_type.eq(Qpipe);

    if netconn1_p || serialconn1_p || pipeconn1_p {
        let childp = process.childp;
        process.set_childp(plist_put(childp, QCsentinel, sentinel));
    }
    sentinel
}

fn pset_sentinel(mut process: LispProcessRef, val: LispObject) -> LispObject {
    let sentinel = if val.is_nil() {
        Qinternal_default_process_sentinel
    } else {
        val
    };
    process.sentinel = sentinel;
    sentinel
}

/// Send PROCESS the contents of STRING as input.
/// PROCESS may be a process, a buffer, the name of a process or buffer, or
/// nil, indicating the current buffer's process.
/// If STRING is more than 500 characters long,
/// it is sent in several bunches.  This may happen even for shorter strings.
/// Output from processes can arrive in between bunches.
///
/// If PROCESS is a non-blocking network process that hasn't been fully
/// set up yet, this function will block until socket setup has completed.
#[lisp_fn]
pub fn process_send_string(process: LispObject, mut string: LispStringRef) {
    unsafe {
        send_process(
            get_process(process),
            string.u.s.data as *mut libc::c_char,
            STRING_BYTES(string.as_mut()),
            string.into(),
        )
    };
}

/// Return the current value of query-on-exit flag for PROCESS.
#[lisp_fn]
pub fn process_query_on_exit_flag(process: LispProcessRef) -> bool {
    !process.kill_without_query()
}

/// Specify if query is needed for PROCESS when Emacs is exited.
/// If the second argument FLAG is non-nil, Emacs will query the user before
/// exiting or killing a buffer if PROCESS is running.  This function
/// returns FLAG.
#[lisp_fn]
pub fn set_process_query_on_exit_flag(mut process: LispProcessRef, flag: LispObject) -> LispObject {
    process.set_kill_without_query(flag.is_nil());
    flag
}

/// Return non-nil if Emacs is waiting for input from the user.
/// This is intended for use by asynchronous process output filters and sentinels.
#[lisp_fn]
pub fn waiting_for_user_input_p() -> bool {
    unsafe { (*current_thread).m_waiting_for_user_input_p != 0 }
}

/// Return the value of inherit-coding-system flag for PROCESS. If this flag is
/// t, `buffer-file-coding-system` of the buffer associated with process will
/// inherit the coding system used to decode the process output.
#[lisp_fn]
pub fn process_inherit_coding_system_flag(process: LispProcessRef) -> bool {
    process.inherit_coding_system_flag()
}

/// Return the exit status of PROCESS or the signal number that killed it.
/// If PROCESS has not yet exited or died, return 0.
#[lisp_fn]
pub fn process_exit_status(mut process: LispProcessRef) -> LispObject {
    if process.raw_status_new() {
        unsafe { update_status(process.as_mut()) };
    }
    let status = process.status;
    match status.into() {
        None => 0.into(),
        Some((_, d)) => car(d),
    }
}

/// Return non-nil if PROCESS has given the terminal to a
/// child.  If the operating system does not make it possible to find out,
/// return t.  If we can find out, return the numeric ID of the foreground
/// process group.
#[lisp_fn(min = "0")]
pub fn process_running_child_p(mut process: LispObject) -> LispObject {
    // Initialize in case ioctl doesn't exist or gives an error,
    // in a way that will cause returning t.
    process = get_process(process);
    let mut proc_ref: LispProcessRef = process.into();

    if !proc_ref.ptype().eq(Qreal) {
        error!("Process {} is not a subprocess.", proc_ref.name);
    }
    if proc_ref.infd < 0 {
        error!("Process {} is not active.", proc_ref.name);
    }

    let gid = unsafe { emacs_get_tty_pgrp(proc_ref.as_mut()) };

    if gid == proc_ref.pid {
        Qnil
    } else if gid == -1 {
        Qt
    } else {
        LispObject::from_fixnum(gid.into())
    }
}

/// Return a list of numerical process IDs of all running processes.
/// If this functionality is unsupported, return nil.
///
/// See `process-attributes' for getting attributes of a process given its ID.
#[lisp_fn(name = "list-system-processes", c_name = "list_system_processes")]
pub fn list_system_processes_lisp() -> LispObject {
    unsafe { list_system_processes() }
}

/// Interrupt process PROCESS
/// PROCESS may be a process, a buffer, or the name of a process or buffer.
/// No arg or nil means current buffer's process.
/// Second arg CURRENT-GROUP non-nil means send signal to
/// the current process-group of the process's controlling terminal
/// rather than to the process's own process group.
/// If the process is a shell, this means interrupt current subjob
/// rather than the shell.
///
/// If CURRENT-GROUP is `lambda', and if the shell owns the terminal,
/// don't send the signal.
///
/// This function calls the functions of `interrupt-process-functions' in
/// the order of the list, until one of them returns non-`nil'.
#[lisp_fn(min = "0")]
pub fn interrupt_process(process: LispObject, current_group: LispObject) -> LispObject {
    run_hook_with_args_until_success(&mut [Qinterrupt_process_functions, process, current_group])
}
def_lisp_sym!(Qinterrupt_process_functions, "interrupt-process-functions");

/// Send a signal number SIGNO to PROCESS.
/// If CURRENT_GROUP is t, that means send to the process group
/// that currently owns the terminal being used to communicate with PROCESS.
/// This is used for various commands in shell mode.
/// If CURRENT_GROUP is lambda, that means send to the process group
/// that currently owns the terminal, but only if it is NOT the shell itself.
///
/// If NOMSG is false, insert signal-announcements into process's buffers
/// right away.
///
/// If we can, we try to signal PROCESS by sending control characters
/// down the pty.  This allows us to signal inferiors who have changed
/// their uid, for which kill would return an EPERM error.
fn process_send_signal(
    process: LispObject,
    signo: libc::c_int,
    mut current_group: LispObject,
    nomsg: bool,
) {
    let gid_: libc::c_int;
    let proc = get_process(process);
    let mut p_ref: LispProcessRef = get_process(process).into();
    let mut no_pgrp: bool = false;

    if !p_ref.ptype().eq(Qreal) {
        error!("Process {} is not a subprocess.", p_ref.name);
    }
    if p_ref.infd < 0 {
        error!("Process {} is not active.", p_ref.name);
    }

    if !p_ref.pty_flag() {
        current_group = Qnil;
    }

    let mut gid: pid_t;

    // If we are using pgrps, get a pgrp number and make it negative.
    if current_group.is_nil() {
        // Send the signal to the shell's process group.
        gid = p_ref.pid;
    } else {
        #[cfg(feature = "signals-via-characters")]
        unsafe {
            let mut termios = mem::uninitialized();
            libc::tcgetattr(p_ref.infd, &mut termios);

            let sig_char = match signo {
                libc::SIGINT => termios.c_cc[libc::VINTR],
                libc::SIGQUIT => termios.c_cc[libc::VQUIT],
                libc::SIGTSTP => termios.c_cc[libc::VSUSP],
                _ => 0,
            };

            if (sig_char != 0) && (sig_char != CDISABLE) {
                send_process(proc, sig_char as *mut libc::c_char, 1, Qnil);
                return;
            }
        }

        // If we can't send the signal with a character,
        // fall through and send it another way.

        // The code above may fall through if it can't
        // handle the signal.

        #[cfg(feature = "tiocgpgrp")]
        // Get the current pgrp using the tty itself, if we have that.
        // Otherwise, use the pty to get the pgrp.
        // On pfa systems, saka@pfu.fujitsu.co.JP writes:
        // "TIOCGPGRP symbol defined in sys/ioctl.h at E50.
        // But, TIOCGPGRP does not work on E50 ;-P works fine on E60"
        // His patch indicates that if TIOCGPGRP returns an error, then
        // we should just assume that p->pid is also the process group id.
        unsafe {
            gid = emacs_get_tty_pgrp(p_ref.as_mut());

            // If we can't get the information, assume
            // the shell owns the tty.
            if gid == -1 {
                gid = p_ref.pid;
            }

            // It is not clear whether anything really can set GID to -1.
            // Perhaps on some system one of those ioctls can or could do so.
            // Or perhaps this is vestigial.
            if gid == -1 {
                no_pgrp = true;
            }
        }
        #[cfg(not(feature = "tiocgpgrp"))]
        {
            gid = p_ref.pid;
        }

        // If current_group is lambda, and the shell owns the terminal,
        // don't send any signal.
        if current_group.eq(Qlambda) && (gid == p_ref.pid) {
            return;
        }
    }

    unsafe {
        if signo == libc::SIGCONT {
            p_ref.set_raw_status_new(false);
            pset_status(p_ref.as_mut(), Qrun);
            p_ref.tick = update_process_tick();
            if !nomsg {
                status_notify(ptr::null_mut(), ptr::null_mut());
                redisplay_preserve_echo_area(13);
            }
        }
    }

    // Work around a HP-UX 7.0 bug that mishandles signals to subjobs.
    // We don't know whether the bug is fixed in later HP-UX versions.
    #[cfg(feature = "tiocsigsend")]
    {
        if current_group.is_not_nil() {
            libc::ioctl(p_ref.infd, libc::TIOCSIGSEND, signo)
        }
    }

    // If we don't have process groups, send the signal to the immediate
    // subprocess. That isn't really right, but it's better than any
    // obvious alternative.
    let pid: pid_t = if no_pgrp { -gid } else { gid };

    // Do not kill an already-reaped process, as that could kill an
    // innocent bystander that happens to have the same process ID.
    unsafe {
        let mut oldset: sigset_t = mem::uninitialized();;
        block_child_signal(&mut oldset);
        if p_ref.alive() {
            libc::kill(pid, signo);
        }
        unblock_child_signal(&mut oldset);
    }
}

/// Default function to interrupt process PROCESS.
/// It shall be the last element in list `interrupt-process-functions'.
/// See function `interrupt-process' for more details on usage.
#[lisp_fn(min = "0")]
pub fn internal_default_interrupt_process(
    process: LispObject,
    current_group: LispObject,
) -> LispObject {
    process_send_signal(process, libc::SIGINT, current_group, false);
    process
}
def_lisp_sym!(
    Qinternal_default_interrupt_process,
    "internal-default-interrupt-process"
);

/// Kill process PROCESS.  May be process or name of one.
/// See function `interrupt-process' for more details on usage.
#[lisp_fn(min = "0")]
pub fn kill_process(process: LispObject, current_group: LispObject) -> LispObject {
    process_send_signal(process, libc::SIGKILL, current_group, false);
    process
}

/// Send QUIT signal to process PROCESS.  May be process or name of one.
/// See function `interrupt-process' for more details on usage.
#[lisp_fn(min = "0")]
pub fn quit_process(process: LispObject, current_group: LispObject) -> LispObject {
    process_send_signal(process, libc::SIGQUIT, current_group, false);
    process
}

/// Stop process PROCESS.  May be process or name of one.
/// See function `interrupt-process' for more details on usage.
/// If PROCESS is a network or serial or pipe connection, inhibit handling
/// of incoming traffic.
#[lisp_fn(min = "0")]
pub fn stop_process(process: LispObject, current_group: LispObject) -> LispObject {
    let mut p_ref: LispProcessRef = get_process(process).into();
    let process_type = p_ref.ptype();
    if process_type.eq(Qnetwork) || process_type.eq(Qserial) || process_type.eq(Qpipe) {
        unsafe {
            delete_read_fd(p_ref.infd);
            p_ref.command = Qt;
        }
        return process;
    }

    process_send_signal(process, libc::SIGTSTP, current_group, false);
    process
}

/// Continue process PROCESS.  May be process or name of one.
/// See function `interrupt-process' for more details on usage.
/// If PROCESS is a network or serial process, resume handling of incoming
/// traffic.
#[lisp_fn(min = "0")]
pub fn continue_process(process: LispObject, current_group: LispObject) -> LispObject {
    let mut p_ref: LispProcessRef = process.into();
    let process_type = p_ref.ptype();
    if process_type.eq(Qnetwork) || process_type.eq(Qserial) || process_type.eq(Qpipe) {
        unsafe {
            if p_ref.command.eq(Qt)
                && p_ref.infd >= 0
                && (!p_ref.filter.eq(Qt) || p_ref.status.eq(Qlisten))
            {
                add_process_read_fd(p_ref.infd);
                tcflush(p_ref.infd, libc::TCIFLUSH);
            }
        }
        p_ref.command = Qnil;
        return process;
    }

    process_send_signal(process, libc::SIGCONT, current_group, false);
    process
}

/// Kill all processes associated with `buffer'.
/// If `buffer' is nil, kill all processes.
#[no_mangle]
pub unsafe extern "C" fn kill_buffer_processes(buffer: LispObject) {
    for_each_process!(p => {
        let p_ref: LispProcessRef = p.into();
        let process_type = p_ref.ptype();
        if process_type.eq(Qnetwork) || process_type.eq(Qserial) || process_type.eq(Qpipe) {
            let process = LispObject::from(p_ref);
            if buffer.is_nil() || p_ref.buffer.eq(buffer) {
                unsafe { Fdelete_process(process); }
            } else if p_ref.infd >= 0 {
                process_send_signal(process, libc::SIGHUP, Qnil, true);
            }
        }
    });
}

include!(concat!(env!("OUT_DIR"), "/process_exports.rs"));
