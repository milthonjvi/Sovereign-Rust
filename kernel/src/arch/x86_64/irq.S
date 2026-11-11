//! Remapped legacy PIC IRQ entry stubs.

.intel_syntax noprefix

.macro IRQ_STUB num
.global sovereign_irq_\num
.type sovereign_irq_\num, @function
sovereign_irq_\num:
    push 0
    push (32 + \num)
    jmp sovereign_irq_common
.endm

IRQ_STUB 0
IRQ_STUB 1
IRQ_STUB 2
IRQ_STUB 3
IRQ_STUB 4
IRQ_STUB 5
IRQ_STUB 6
IRQ_STUB 7
IRQ_STUB 8
IRQ_STUB 9
IRQ_STUB 10
IRQ_STUB 11
IRQ_STUB 12
IRQ_STUB 13
IRQ_STUB 14
IRQ_STUB 15

.global sovereign_irq_stub_table
.type sovereign_irq_stub_table, @object
.align 8
sovereign_irq_stub_table:
.quad sovereign_irq_0, sovereign_irq_1, sovereign_irq_2, sovereign_irq_3
.quad sovereign_irq_4, sovereign_irq_5, sovereign_irq_6, sovereign_irq_7
.quad sovereign_irq_8, sovereign_irq_9, sovereign_irq_10, sovereign_irq_11
.quad sovereign_irq_12, sovereign_irq_13, sovereign_irq_14, sovereign_irq_15
.size sovereign_irq_stub_table, .-sovereign_irq_stub_table

.global sovereign_irq_common
.type sovereign_irq_common, @function
sovereign_irq_common:
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

    // Keep the original frame pointer across ABI stack alignment and the call.
    mov r12, rsp
    mov rdi, rsp
    and rsp, -16
    call sovereign_irq_dispatch
    mov rsp, r12

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    iretq
.size sovereign_irq_common, .-sovereign_irq_common
