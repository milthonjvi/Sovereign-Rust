# x86_64 exception entry stubs.
# The common path captures general-purpose registers and transfers to Rust.
# Exception dispatch is deliberately fatal until diagnostics and recovery are ready.

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
    # The processor already pushed the exception error code.
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

ISR_NOERR 32
ISR_NOERR 33
ISR_NOERR 34
ISR_NOERR 35
ISR_NOERR 36
ISR_NOERR 37
ISR_NOERR 38
ISR_NOERR 39
ISR_NOERR 40
ISR_NOERR 41
ISR_NOERR 42
ISR_NOERR 43
ISR_NOERR 44
ISR_NOERR 45
ISR_NOERR 46
ISR_NOERR 47
ISR_NOERR 48
ISR_NOERR 49
ISR_NOERR 50
ISR_NOERR 51
ISR_NOERR 52
ISR_NOERR 53
ISR_NOERR 54
ISR_NOERR 55
ISR_NOERR 56
ISR_NOERR 57
ISR_NOERR 58
ISR_NOERR 59
ISR_NOERR 60
ISR_NOERR 61
ISR_NOERR 62
ISR_NOERR 63
ISR_NOERR 64
ISR_NOERR 65
ISR_NOERR 66
ISR_NOERR 67
ISR_NOERR 68
ISR_NOERR 69
ISR_NOERR 70
ISR_NOERR 71
ISR_NOERR 72
ISR_NOERR 73
ISR_NOERR 74
ISR_NOERR 75
ISR_NOERR 76
ISR_NOERR 77
ISR_NOERR 78
ISR_NOERR 79
ISR_NOERR 80
ISR_NOERR 81
ISR_NOERR 82
ISR_NOERR 83
ISR_NOERR 84
ISR_NOERR 85
ISR_NOERR 86
ISR_NOERR 87
ISR_NOERR 88
ISR_NOERR 89
ISR_NOERR 90
ISR_NOERR 91
ISR_NOERR 92
ISR_NOERR 93
ISR_NOERR 94
ISR_NOERR 95
ISR_NOERR 96
ISR_NOERR 97
ISR_NOERR 98
ISR_NOERR 99
ISR_NOERR 100
ISR_NOERR 101
ISR_NOERR 102
ISR_NOERR 103
ISR_NOERR 104
ISR_NOERR 105
ISR_NOERR 106
ISR_NOERR 107
ISR_NOERR 108
ISR_NOERR 109
ISR_NOERR 110
ISR_NOERR 111
ISR_NOERR 112
ISR_NOERR 113
ISR_NOERR 114
ISR_NOERR 115
ISR_NOERR 116
ISR_NOERR 117
ISR_NOERR 118
ISR_NOERR 119
ISR_NOERR 120
ISR_NOERR 121
ISR_NOERR 122
ISR_NOERR 123
ISR_NOERR 124
ISR_NOERR 125
ISR_NOERR 126
ISR_NOERR 127
ISR_NOERR 128
ISR_NOERR 129
ISR_NOERR 130
ISR_NOERR 131
ISR_NOERR 132
ISR_NOERR 133
ISR_NOERR 134
ISR_NOERR 135
ISR_NOERR 136
ISR_NOERR 137
ISR_NOERR 138
ISR_NOERR 139
ISR_NOERR 140
ISR_NOERR 141
ISR_NOERR 142
ISR_NOERR 143
ISR_NOERR 144
ISR_NOERR 145
ISR_NOERR 146
ISR_NOERR 147
ISR_NOERR 148
ISR_NOERR 149
ISR_NOERR 150
ISR_NOERR 151
ISR_NOERR 152
ISR_NOERR 153
ISR_NOERR 154
ISR_NOERR 155
ISR_NOERR 156
ISR_NOERR 157
ISR_NOERR 158
ISR_NOERR 159
ISR_NOERR 160
ISR_NOERR 161
ISR_NOERR 162
ISR_NOERR 163
ISR_NOERR 164
ISR_NOERR 165
ISR_NOERR 166
ISR_NOERR 167
ISR_NOERR 168
ISR_NOERR 169
ISR_NOERR 170
ISR_NOERR 171
ISR_NOERR 172
ISR_NOERR 173
ISR_NOERR 174
ISR_NOERR 175
ISR_NOERR 176
ISR_NOERR 177
ISR_NOERR 178
ISR_NOERR 179
ISR_NOERR 180
ISR_NOERR 181
ISR_NOERR 182
ISR_NOERR 183
ISR_NOERR 184
ISR_NOERR 185
ISR_NOERR 186
ISR_NOERR 187
ISR_NOERR 188
ISR_NOERR 189
ISR_NOERR 190
ISR_NOERR 191
ISR_NOERR 192
ISR_NOERR 193
ISR_NOERR 194
ISR_NOERR 195
ISR_NOERR 196
ISR_NOERR 197
ISR_NOERR 198
ISR_NOERR 199
ISR_NOERR 200
ISR_NOERR 201
ISR_NOERR 202
ISR_NOERR 203
ISR_NOERR 204
ISR_NOERR 205
ISR_NOERR 206
ISR_NOERR 207
ISR_NOERR 208
ISR_NOERR 209
ISR_NOERR 210
ISR_NOERR 211
ISR_NOERR 212
ISR_NOERR 213
ISR_NOERR 214
ISR_NOERR 215
ISR_NOERR 216
ISR_NOERR 217
ISR_NOERR 218
ISR_NOERR 219
ISR_NOERR 220
ISR_NOERR 221
ISR_NOERR 222
ISR_NOERR 223
ISR_NOERR 224
ISR_NOERR 225
ISR_NOERR 226
ISR_NOERR 227
ISR_NOERR 228
ISR_NOERR 229
ISR_NOERR 230
ISR_NOERR 231
ISR_NOERR 232
ISR_NOERR 233
ISR_NOERR 234
ISR_NOERR 235
ISR_NOERR 236
ISR_NOERR 237
ISR_NOERR 238
ISR_NOERR 239
ISR_NOERR 240
ISR_NOERR 241
ISR_NOERR 242
ISR_NOERR 243
ISR_NOERR 244
ISR_NOERR 245
ISR_NOERR 246
ISR_NOERR 247
ISR_NOERR 248
ISR_NOERR 249
ISR_NOERR 250
ISR_NOERR 251
ISR_NOERR 252
ISR_NOERR 253
ISR_NOERR 254
ISR_NOERR 255

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
.quad sovereign_isr_32, sovereign_isr_33, sovereign_isr_34, sovereign_isr_35
.quad sovereign_isr_36, sovereign_isr_37, sovereign_isr_38, sovereign_isr_39
.quad sovereign_isr_40, sovereign_isr_41, sovereign_isr_42, sovereign_isr_43
.quad sovereign_isr_44, sovereign_isr_45, sovereign_isr_46, sovereign_isr_47
.quad sovereign_isr_48, sovereign_isr_49, sovereign_isr_50, sovereign_isr_51
.quad sovereign_isr_52, sovereign_isr_53, sovereign_isr_54, sovereign_isr_55
.quad sovereign_isr_56, sovereign_isr_57, sovereign_isr_58, sovereign_isr_59
.quad sovereign_isr_60, sovereign_isr_61, sovereign_isr_62, sovereign_isr_63
.quad sovereign_isr_64, sovereign_isr_65, sovereign_isr_66, sovereign_isr_67
.quad sovereign_isr_68, sovereign_isr_69, sovereign_isr_70, sovereign_isr_71
.quad sovereign_isr_72, sovereign_isr_73, sovereign_isr_74, sovereign_isr_75
.quad sovereign_isr_76, sovereign_isr_77, sovereign_isr_78, sovereign_isr_79
.quad sovereign_isr_80, sovereign_isr_81, sovereign_isr_82, sovereign_isr_83
.quad sovereign_isr_84, sovereign_isr_85, sovereign_isr_86, sovereign_isr_87
.quad sovereign_isr_88, sovereign_isr_89, sovereign_isr_90, sovereign_isr_91
.quad sovereign_isr_92, sovereign_isr_93, sovereign_isr_94, sovereign_isr_95
.quad sovereign_isr_96, sovereign_isr_97, sovereign_isr_98, sovereign_isr_99
.quad sovereign_isr_100, sovereign_isr_101, sovereign_isr_102, sovereign_isr_103
.quad sovereign_isr_104, sovereign_isr_105, sovereign_isr_106, sovereign_isr_107
.quad sovereign_isr_108, sovereign_isr_109, sovereign_isr_110, sovereign_isr_111
.quad sovereign_isr_112, sovereign_isr_113, sovereign_isr_114, sovereign_isr_115
.quad sovereign_isr_116, sovereign_isr_117, sovereign_isr_118, sovereign_isr_119
.quad sovereign_isr_120, sovereign_isr_121, sovereign_isr_122, sovereign_isr_123
.quad sovereign_isr_124, sovereign_isr_125, sovereign_isr_126, sovereign_isr_127
.quad sovereign_isr_128, sovereign_isr_129, sovereign_isr_130, sovereign_isr_131
.quad sovereign_isr_132, sovereign_isr_133, sovereign_isr_134, sovereign_isr_135
.quad sovereign_isr_136, sovereign_isr_137, sovereign_isr_138, sovereign_isr_139
.quad sovereign_isr_140, sovereign_isr_141, sovereign_isr_142, sovereign_isr_143
.quad sovereign_isr_144, sovereign_isr_145, sovereign_isr_146, sovereign_isr_147
.quad sovereign_isr_148, sovereign_isr_149, sovereign_isr_150, sovereign_isr_151
.quad sovereign_isr_152, sovereign_isr_153, sovereign_isr_154, sovereign_isr_155
.quad sovereign_isr_156, sovereign_isr_157, sovereign_isr_158, sovereign_isr_159
.quad sovereign_isr_160, sovereign_isr_161, sovereign_isr_162, sovereign_isr_163
.quad sovereign_isr_164, sovereign_isr_165, sovereign_isr_166, sovereign_isr_167
.quad sovereign_isr_168, sovereign_isr_169, sovereign_isr_170, sovereign_isr_171
.quad sovereign_isr_172, sovereign_isr_173, sovereign_isr_174, sovereign_isr_175
.quad sovereign_isr_176, sovereign_isr_177, sovereign_isr_178, sovereign_isr_179
.quad sovereign_isr_180, sovereign_isr_181, sovereign_isr_182, sovereign_isr_183
.quad sovereign_isr_184, sovereign_isr_185, sovereign_isr_186, sovereign_isr_187
.quad sovereign_isr_188, sovereign_isr_189, sovereign_isr_190, sovereign_isr_191
.quad sovereign_isr_192, sovereign_isr_193, sovereign_isr_194, sovereign_isr_195
.quad sovereign_isr_196, sovereign_isr_197, sovereign_isr_198, sovereign_isr_199
.quad sovereign_isr_200, sovereign_isr_201, sovereign_isr_202, sovereign_isr_203
.quad sovereign_isr_204, sovereign_isr_205, sovereign_isr_206, sovereign_isr_207
.quad sovereign_isr_208, sovereign_isr_209, sovereign_isr_210, sovereign_isr_211
.quad sovereign_isr_212, sovereign_isr_213, sovereign_isr_214, sovereign_isr_215
.quad sovereign_isr_216, sovereign_isr_217, sovereign_isr_218, sovereign_isr_219
.quad sovereign_isr_220, sovereign_isr_221, sovereign_isr_222, sovereign_isr_223
.quad sovereign_isr_224, sovereign_isr_225, sovereign_isr_226, sovereign_isr_227
.quad sovereign_isr_228, sovereign_isr_229, sovereign_isr_230, sovereign_isr_231
.quad sovereign_isr_232, sovereign_isr_233, sovereign_isr_234, sovereign_isr_235
.quad sovereign_isr_236, sovereign_isr_237, sovereign_isr_238, sovereign_isr_239
.quad sovereign_isr_240, sovereign_isr_241, sovereign_isr_242, sovereign_isr_243
.quad sovereign_isr_244, sovereign_isr_245, sovereign_isr_246, sovereign_isr_247
.quad sovereign_isr_248, sovereign_isr_249, sovereign_isr_250, sovereign_isr_251
.quad sovereign_isr_252, sovereign_isr_253, sovereign_isr_254, sovereign_isr_255
.size sovereign_exception_stub_table, .-sovereign_exception_stub_table

.global sovereign_exception_common
.type sovereign_exception_common, @function
sovereign_exception_common:
    # Save all general-purpose registers. The stack layout is ExceptionFrame.
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

    # Rust's x86-64 ABI requires the direction flag to be clear.
    cld
    mov rdi, rsp
    and rsp, -16
    call sovereign_exception_dispatch

.Lexception_halt:
    cli
    hlt
    jmp .Lexception_halt
.size sovereign_exception_common, .-sovereign_exception_common

# APIC spurious vector: no EOI is sent for this vector by architectural design.
# The interrupt gate already saved RIP, CS and RFLAGS; IRETQ restores them.
.global sovereign_apic_spurious_stub
.type sovereign_apic_spurious_stub, @function
sovereign_apic_spurious_stub:
    iretq
.size sovereign_apic_spurious_stub, .-sovereign_apic_spurious_stub
