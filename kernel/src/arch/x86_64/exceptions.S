//! x86_64 exception entry stubs.
//! The common path captures general-purpose registers and transfers to Rust.
//! Exception dispatch is deliberately fatal until diagnostics and recovery are ready.

.intel_syntax noprefix

.macro ISR_NOERR num
.global sovereign_isr_\num
.type sovereign_isr_\num, @function
sovereign_isr_\num:
    push 0
    push \num
    jmp sovereign_exception_common
.endm

.macro ISR_ERR num
.global sovereign_isr_\num
.type sovereign_isr_\num, @function
sovereign_isr_\num:
    // The processor already pushed the exception error code.
    push \num
    jmp sovereign_exception_common
.endm

ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR   8
ISR_NOERR 9
ISR_ERR   10
ISR_ERR   11
ISR_ERR   12
ISR_ERR   13
ISR_ERR   14
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR   17
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_ERR   21
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_ERR   29
ISR_ERR   30
ISR_NOERR 31

.global sovereign_exception_stub_table
.type sovereign_exception_stub_table, @object
.align 8
sovereign_exception_stub_table:
.quad sovereign_isr_0, sovereign_isr_1, sovereign_isr_2, sovereign_isr_3
.quad sovereign_isr_4, sovereign_isr_5, sovereign_isr_6, sovereign_isr_7
.quad sovereign_isr_8, sovereign_isr_9, sovereign_isr_10, sovereign_isr_11
.quad sovereign_isr_12, sovereign_isr_13, sovereign_isr_14, sovereign_isr_15
.quad sovereign_isr_16, sovereign_isr_17, sovereign_isr_18, sovereign_isr_19
.quad sovereign_isr_20, sovereign_isr_21, sovereign_isr_22, sovereign_isr_23
.quad sovereign_isr_24, sovereign_isr_25, sovereign_isr_26, sovereign_isr_27
.quad sovereign_isr_28, sovereign_isr_29, sovereign_isr_30, sovereign_isr_31
.size sovereign_exception_stub_table, .-sovereign_exception_stub_table

.global sovereign_exception_common
.type sovereign_exception_common, @function
sovereign_exception_common:
    // Save all general-purpose registers. The stack layout is ExceptionFrame.
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    mov rdi, rsp
    and rsp, -16
    call sovereign_exception_dispatch

.Lexception_halt:
    cli
    hlt
    jmp .Lexception_halt
.size sovereign_exception_common, .-sovereign_exception_common
