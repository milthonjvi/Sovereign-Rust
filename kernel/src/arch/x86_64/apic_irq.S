# x2APIC/IOAPIC interrupt stubs. Vectors 0x30..=0xFE except 0x80.
# 0x80 is reserved for the future syscall ABI; 0xFF is the APIC spurious vector.
.intel_syntax noprefix

.global sovereign_apic_isr_48
.type sovereign_apic_isr_48, @function
sovereign_apic_isr_48:
    push 0
    push 48
    jmp sovereign_apic_common
.size sovereign_apic_isr_48, .-sovereign_apic_isr_48

.global sovereign_apic_isr_49
.type sovereign_apic_isr_49, @function
sovereign_apic_isr_49:
    push 0
    push 49
    jmp sovereign_apic_common
.size sovereign_apic_isr_49, .-sovereign_apic_isr_49

.global sovereign_apic_isr_50
.type sovereign_apic_isr_50, @function
sovereign_apic_isr_50:
    push 0
    push 50
    jmp sovereign_apic_common
.size sovereign_apic_isr_50, .-sovereign_apic_isr_50

.global sovereign_apic_isr_51
.type sovereign_apic_isr_51, @function
sovereign_apic_isr_51:
    push 0
    push 51
    jmp sovereign_apic_common
.size sovereign_apic_isr_51, .-sovereign_apic_isr_51

.global sovereign_apic_isr_52
.type sovereign_apic_isr_52, @function
sovereign_apic_isr_52:
    push 0
    push 52
    jmp sovereign_apic_common
.size sovereign_apic_isr_52, .-sovereign_apic_isr_52

.global sovereign_apic_isr_53
.type sovereign_apic_isr_53, @function
sovereign_apic_isr_53:
    push 0
    push 53
    jmp sovereign_apic_common
.size sovereign_apic_isr_53, .-sovereign_apic_isr_53

.global sovereign_apic_isr_54
.type sovereign_apic_isr_54, @function
sovereign_apic_isr_54:
    push 0
    push 54
    jmp sovereign_apic_common
.size sovereign_apic_isr_54, .-sovereign_apic_isr_54

.global sovereign_apic_isr_55
.type sovereign_apic_isr_55, @function
sovereign_apic_isr_55:
    push 0
    push 55
    jmp sovereign_apic_common
.size sovereign_apic_isr_55, .-sovereign_apic_isr_55

.global sovereign_apic_isr_56
.type sovereign_apic_isr_56, @function
sovereign_apic_isr_56:
    push 0
    push 56
    jmp sovereign_apic_common
.size sovereign_apic_isr_56, .-sovereign_apic_isr_56

.global sovereign_apic_isr_57
.type sovereign_apic_isr_57, @function
sovereign_apic_isr_57:
    push 0
    push 57
    jmp sovereign_apic_common
.size sovereign_apic_isr_57, .-sovereign_apic_isr_57

.global sovereign_apic_isr_58
.type sovereign_apic_isr_58, @function
sovereign_apic_isr_58:
    push 0
    push 58
    jmp sovereign_apic_common
.size sovereign_apic_isr_58, .-sovereign_apic_isr_58

.global sovereign_apic_isr_59
.type sovereign_apic_isr_59, @function
sovereign_apic_isr_59:
    push 0
    push 59
    jmp sovereign_apic_common
.size sovereign_apic_isr_59, .-sovereign_apic_isr_59

.global sovereign_apic_isr_60
.type sovereign_apic_isr_60, @function
sovereign_apic_isr_60:
    push 0
    push 60
    jmp sovereign_apic_common
.size sovereign_apic_isr_60, .-sovereign_apic_isr_60

.global sovereign_apic_isr_61
.type sovereign_apic_isr_61, @function
sovereign_apic_isr_61:
    push 0
    push 61
    jmp sovereign_apic_common
.size sovereign_apic_isr_61, .-sovereign_apic_isr_61

.global sovereign_apic_isr_62
.type sovereign_apic_isr_62, @function
sovereign_apic_isr_62:
    push 0
    push 62
    jmp sovereign_apic_common
.size sovereign_apic_isr_62, .-sovereign_apic_isr_62

.global sovereign_apic_isr_63
.type sovereign_apic_isr_63, @function
sovereign_apic_isr_63:
    push 0
    push 63
    jmp sovereign_apic_common
.size sovereign_apic_isr_63, .-sovereign_apic_isr_63

.global sovereign_apic_isr_64
.type sovereign_apic_isr_64, @function
sovereign_apic_isr_64:
    push 0
    push 64
    jmp sovereign_apic_common
.size sovereign_apic_isr_64, .-sovereign_apic_isr_64

.global sovereign_apic_isr_65
.type sovereign_apic_isr_65, @function
sovereign_apic_isr_65:
    push 0
    push 65
    jmp sovereign_apic_common
.size sovereign_apic_isr_65, .-sovereign_apic_isr_65

.global sovereign_apic_isr_66
.type sovereign_apic_isr_66, @function
sovereign_apic_isr_66:
    push 0
    push 66
    jmp sovereign_apic_common
.size sovereign_apic_isr_66, .-sovereign_apic_isr_66

.global sovereign_apic_isr_67
.type sovereign_apic_isr_67, @function
sovereign_apic_isr_67:
    push 0
    push 67
    jmp sovereign_apic_common
.size sovereign_apic_isr_67, .-sovereign_apic_isr_67

.global sovereign_apic_isr_68
.type sovereign_apic_isr_68, @function
sovereign_apic_isr_68:
    push 0
    push 68
    jmp sovereign_apic_common
.size sovereign_apic_isr_68, .-sovereign_apic_isr_68

.global sovereign_apic_isr_69
.type sovereign_apic_isr_69, @function
sovereign_apic_isr_69:
    push 0
    push 69
    jmp sovereign_apic_common
.size sovereign_apic_isr_69, .-sovereign_apic_isr_69

.global sovereign_apic_isr_70
.type sovereign_apic_isr_70, @function
sovereign_apic_isr_70:
    push 0
    push 70
    jmp sovereign_apic_common
.size sovereign_apic_isr_70, .-sovereign_apic_isr_70

.global sovereign_apic_isr_71
.type sovereign_apic_isr_71, @function
sovereign_apic_isr_71:
    push 0
    push 71
    jmp sovereign_apic_common
.size sovereign_apic_isr_71, .-sovereign_apic_isr_71

.global sovereign_apic_isr_72
.type sovereign_apic_isr_72, @function
sovereign_apic_isr_72:
    push 0
    push 72
    jmp sovereign_apic_common
.size sovereign_apic_isr_72, .-sovereign_apic_isr_72

.global sovereign_apic_isr_73
.type sovereign_apic_isr_73, @function
sovereign_apic_isr_73:
    push 0
    push 73
    jmp sovereign_apic_common
.size sovereign_apic_isr_73, .-sovereign_apic_isr_73

.global sovereign_apic_isr_74
.type sovereign_apic_isr_74, @function
sovereign_apic_isr_74:
    push 0
    push 74
    jmp sovereign_apic_common
.size sovereign_apic_isr_74, .-sovereign_apic_isr_74

.global sovereign_apic_isr_75
.type sovereign_apic_isr_75, @function
sovereign_apic_isr_75:
    push 0
    push 75
    jmp sovereign_apic_common
.size sovereign_apic_isr_75, .-sovereign_apic_isr_75

.global sovereign_apic_isr_76
.type sovereign_apic_isr_76, @function
sovereign_apic_isr_76:
    push 0
    push 76
    jmp sovereign_apic_common
.size sovereign_apic_isr_76, .-sovereign_apic_isr_76

.global sovereign_apic_isr_77
.type sovereign_apic_isr_77, @function
sovereign_apic_isr_77:
    push 0
    push 77
    jmp sovereign_apic_common
.size sovereign_apic_isr_77, .-sovereign_apic_isr_77

.global sovereign_apic_isr_78
.type sovereign_apic_isr_78, @function
sovereign_apic_isr_78:
    push 0
    push 78
    jmp sovereign_apic_common
.size sovereign_apic_isr_78, .-sovereign_apic_isr_78

.global sovereign_apic_isr_79
.type sovereign_apic_isr_79, @function
sovereign_apic_isr_79:
    push 0
    push 79
    jmp sovereign_apic_common
.size sovereign_apic_isr_79, .-sovereign_apic_isr_79

.global sovereign_apic_isr_80
.type sovereign_apic_isr_80, @function
sovereign_apic_isr_80:
    push 0
    push 80
    jmp sovereign_apic_common
.size sovereign_apic_isr_80, .-sovereign_apic_isr_80

.global sovereign_apic_isr_81
.type sovereign_apic_isr_81, @function
sovereign_apic_isr_81:
    push 0
    push 81
    jmp sovereign_apic_common
.size sovereign_apic_isr_81, .-sovereign_apic_isr_81

.global sovereign_apic_isr_82
.type sovereign_apic_isr_82, @function
sovereign_apic_isr_82:
    push 0
    push 82
    jmp sovereign_apic_common
.size sovereign_apic_isr_82, .-sovereign_apic_isr_82

.global sovereign_apic_isr_83
.type sovereign_apic_isr_83, @function
sovereign_apic_isr_83:
    push 0
    push 83
    jmp sovereign_apic_common
.size sovereign_apic_isr_83, .-sovereign_apic_isr_83

.global sovereign_apic_isr_84
.type sovereign_apic_isr_84, @function
sovereign_apic_isr_84:
    push 0
    push 84
    jmp sovereign_apic_common
.size sovereign_apic_isr_84, .-sovereign_apic_isr_84

.global sovereign_apic_isr_85
.type sovereign_apic_isr_85, @function
sovereign_apic_isr_85:
    push 0
    push 85
    jmp sovereign_apic_common
.size sovereign_apic_isr_85, .-sovereign_apic_isr_85

.global sovereign_apic_isr_86
.type sovereign_apic_isr_86, @function
sovereign_apic_isr_86:
    push 0
    push 86
    jmp sovereign_apic_common
.size sovereign_apic_isr_86, .-sovereign_apic_isr_86

.global sovereign_apic_isr_87
.type sovereign_apic_isr_87, @function
sovereign_apic_isr_87:
    push 0
    push 87
    jmp sovereign_apic_common
.size sovereign_apic_isr_87, .-sovereign_apic_isr_87

.global sovereign_apic_isr_88
.type sovereign_apic_isr_88, @function
sovereign_apic_isr_88:
    push 0
    push 88
    jmp sovereign_apic_common
.size sovereign_apic_isr_88, .-sovereign_apic_isr_88

.global sovereign_apic_isr_89
.type sovereign_apic_isr_89, @function
sovereign_apic_isr_89:
    push 0
    push 89
    jmp sovereign_apic_common
.size sovereign_apic_isr_89, .-sovereign_apic_isr_89

.global sovereign_apic_isr_90
.type sovereign_apic_isr_90, @function
sovereign_apic_isr_90:
    push 0
    push 90
    jmp sovereign_apic_common
.size sovereign_apic_isr_90, .-sovereign_apic_isr_90

.global sovereign_apic_isr_91
.type sovereign_apic_isr_91, @function
sovereign_apic_isr_91:
    push 0
    push 91
    jmp sovereign_apic_common
.size sovereign_apic_isr_91, .-sovereign_apic_isr_91

.global sovereign_apic_isr_92
.type sovereign_apic_isr_92, @function
sovereign_apic_isr_92:
    push 0
    push 92
    jmp sovereign_apic_common
.size sovereign_apic_isr_92, .-sovereign_apic_isr_92

.global sovereign_apic_isr_93
.type sovereign_apic_isr_93, @function
sovereign_apic_isr_93:
    push 0
    push 93
    jmp sovereign_apic_common
.size sovereign_apic_isr_93, .-sovereign_apic_isr_93

.global sovereign_apic_isr_94
.type sovereign_apic_isr_94, @function
sovereign_apic_isr_94:
    push 0
    push 94
    jmp sovereign_apic_common
.size sovereign_apic_isr_94, .-sovereign_apic_isr_94

.global sovereign_apic_isr_95
.type sovereign_apic_isr_95, @function
sovereign_apic_isr_95:
    push 0
    push 95
    jmp sovereign_apic_common
.size sovereign_apic_isr_95, .-sovereign_apic_isr_95

.global sovereign_apic_isr_96
.type sovereign_apic_isr_96, @function
sovereign_apic_isr_96:
    push 0
    push 96
    jmp sovereign_apic_common
.size sovereign_apic_isr_96, .-sovereign_apic_isr_96

.global sovereign_apic_isr_97
.type sovereign_apic_isr_97, @function
sovereign_apic_isr_97:
    push 0
    push 97
    jmp sovereign_apic_common
.size sovereign_apic_isr_97, .-sovereign_apic_isr_97

.global sovereign_apic_isr_98
.type sovereign_apic_isr_98, @function
sovereign_apic_isr_98:
    push 0
    push 98
    jmp sovereign_apic_common
.size sovereign_apic_isr_98, .-sovereign_apic_isr_98

.global sovereign_apic_isr_99
.type sovereign_apic_isr_99, @function
sovereign_apic_isr_99:
    push 0
    push 99
    jmp sovereign_apic_common
.size sovereign_apic_isr_99, .-sovereign_apic_isr_99

.global sovereign_apic_isr_100
.type sovereign_apic_isr_100, @function
sovereign_apic_isr_100:
    push 0
    push 100
    jmp sovereign_apic_common
.size sovereign_apic_isr_100, .-sovereign_apic_isr_100

.global sovereign_apic_isr_101
.type sovereign_apic_isr_101, @function
sovereign_apic_isr_101:
    push 0
    push 101
    jmp sovereign_apic_common
.size sovereign_apic_isr_101, .-sovereign_apic_isr_101

.global sovereign_apic_isr_102
.type sovereign_apic_isr_102, @function
sovereign_apic_isr_102:
    push 0
    push 102
    jmp sovereign_apic_common
.size sovereign_apic_isr_102, .-sovereign_apic_isr_102

.global sovereign_apic_isr_103
.type sovereign_apic_isr_103, @function
sovereign_apic_isr_103:
    push 0
    push 103
    jmp sovereign_apic_common
.size sovereign_apic_isr_103, .-sovereign_apic_isr_103

.global sovereign_apic_isr_104
.type sovereign_apic_isr_104, @function
sovereign_apic_isr_104:
    push 0
    push 104
    jmp sovereign_apic_common
.size sovereign_apic_isr_104, .-sovereign_apic_isr_104

.global sovereign_apic_isr_105
.type sovereign_apic_isr_105, @function
sovereign_apic_isr_105:
    push 0
    push 105
    jmp sovereign_apic_common
.size sovereign_apic_isr_105, .-sovereign_apic_isr_105

.global sovereign_apic_isr_106
.type sovereign_apic_isr_106, @function
sovereign_apic_isr_106:
    push 0
    push 106
    jmp sovereign_apic_common
.size sovereign_apic_isr_106, .-sovereign_apic_isr_106

.global sovereign_apic_isr_107
.type sovereign_apic_isr_107, @function
sovereign_apic_isr_107:
    push 0
    push 107
    jmp sovereign_apic_common
.size sovereign_apic_isr_107, .-sovereign_apic_isr_107

.global sovereign_apic_isr_108
.type sovereign_apic_isr_108, @function
sovereign_apic_isr_108:
    push 0
    push 108
    jmp sovereign_apic_common
.size sovereign_apic_isr_108, .-sovereign_apic_isr_108

.global sovereign_apic_isr_109
.type sovereign_apic_isr_109, @function
sovereign_apic_isr_109:
    push 0
    push 109
    jmp sovereign_apic_common
.size sovereign_apic_isr_109, .-sovereign_apic_isr_109

.global sovereign_apic_isr_110
.type sovereign_apic_isr_110, @function
sovereign_apic_isr_110:
    push 0
    push 110
    jmp sovereign_apic_common
.size sovereign_apic_isr_110, .-sovereign_apic_isr_110

.global sovereign_apic_isr_111
.type sovereign_apic_isr_111, @function
sovereign_apic_isr_111:
    push 0
    push 111
    jmp sovereign_apic_common
.size sovereign_apic_isr_111, .-sovereign_apic_isr_111

.global sovereign_apic_isr_112
.type sovereign_apic_isr_112, @function
sovereign_apic_isr_112:
    push 0
    push 112
    jmp sovereign_apic_common
.size sovereign_apic_isr_112, .-sovereign_apic_isr_112

.global sovereign_apic_isr_113
.type sovereign_apic_isr_113, @function
sovereign_apic_isr_113:
    push 0
    push 113
    jmp sovereign_apic_common
.size sovereign_apic_isr_113, .-sovereign_apic_isr_113

.global sovereign_apic_isr_114
.type sovereign_apic_isr_114, @function
sovereign_apic_isr_114:
    push 0
    push 114
    jmp sovereign_apic_common
.size sovereign_apic_isr_114, .-sovereign_apic_isr_114

.global sovereign_apic_isr_115
.type sovereign_apic_isr_115, @function
sovereign_apic_isr_115:
    push 0
    push 115
    jmp sovereign_apic_common
.size sovereign_apic_isr_115, .-sovereign_apic_isr_115

.global sovereign_apic_isr_116
.type sovereign_apic_isr_116, @function
sovereign_apic_isr_116:
    push 0
    push 116
    jmp sovereign_apic_common
.size sovereign_apic_isr_116, .-sovereign_apic_isr_116

.global sovereign_apic_isr_117
.type sovereign_apic_isr_117, @function
sovereign_apic_isr_117:
    push 0
    push 117
    jmp sovereign_apic_common
.size sovereign_apic_isr_117, .-sovereign_apic_isr_117

.global sovereign_apic_isr_118
.type sovereign_apic_isr_118, @function
sovereign_apic_isr_118:
    push 0
    push 118
    jmp sovereign_apic_common
.size sovereign_apic_isr_118, .-sovereign_apic_isr_118

.global sovereign_apic_isr_119
.type sovereign_apic_isr_119, @function
sovereign_apic_isr_119:
    push 0
    push 119
    jmp sovereign_apic_common
.size sovereign_apic_isr_119, .-sovereign_apic_isr_119

.global sovereign_apic_isr_120
.type sovereign_apic_isr_120, @function
sovereign_apic_isr_120:
    push 0
    push 120
    jmp sovereign_apic_common
.size sovereign_apic_isr_120, .-sovereign_apic_isr_120

.global sovereign_apic_isr_121
.type sovereign_apic_isr_121, @function
sovereign_apic_isr_121:
    push 0
    push 121
    jmp sovereign_apic_common
.size sovereign_apic_isr_121, .-sovereign_apic_isr_121

.global sovereign_apic_isr_122
.type sovereign_apic_isr_122, @function
sovereign_apic_isr_122:
    push 0
    push 122
    jmp sovereign_apic_common
.size sovereign_apic_isr_122, .-sovereign_apic_isr_122

.global sovereign_apic_isr_123
.type sovereign_apic_isr_123, @function
sovereign_apic_isr_123:
    push 0
    push 123
    jmp sovereign_apic_common
.size sovereign_apic_isr_123, .-sovereign_apic_isr_123

.global sovereign_apic_isr_124
.type sovereign_apic_isr_124, @function
sovereign_apic_isr_124:
    push 0
    push 124
    jmp sovereign_apic_common
.size sovereign_apic_isr_124, .-sovereign_apic_isr_124

.global sovereign_apic_isr_125
.type sovereign_apic_isr_125, @function
sovereign_apic_isr_125:
    push 0
    push 125
    jmp sovereign_apic_common
.size sovereign_apic_isr_125, .-sovereign_apic_isr_125

.global sovereign_apic_isr_126
.type sovereign_apic_isr_126, @function
sovereign_apic_isr_126:
    push 0
    push 126
    jmp sovereign_apic_common
.size sovereign_apic_isr_126, .-sovereign_apic_isr_126

.global sovereign_apic_isr_127
.type sovereign_apic_isr_127, @function
sovereign_apic_isr_127:
    push 0
    push 127
    jmp sovereign_apic_common
.size sovereign_apic_isr_127, .-sovereign_apic_isr_127

.global sovereign_apic_isr_129
.type sovereign_apic_isr_129, @function
sovereign_apic_isr_129:
    push 0
    push 129
    jmp sovereign_apic_common
.size sovereign_apic_isr_129, .-sovereign_apic_isr_129

.global sovereign_apic_isr_130
.type sovereign_apic_isr_130, @function
sovereign_apic_isr_130:
    push 0
    push 130
    jmp sovereign_apic_common
.size sovereign_apic_isr_130, .-sovereign_apic_isr_130

.global sovereign_apic_isr_131
.type sovereign_apic_isr_131, @function
sovereign_apic_isr_131:
    push 0
    push 131
    jmp sovereign_apic_common
.size sovereign_apic_isr_131, .-sovereign_apic_isr_131

.global sovereign_apic_isr_132
.type sovereign_apic_isr_132, @function
sovereign_apic_isr_132:
    push 0
    push 132
    jmp sovereign_apic_common
.size sovereign_apic_isr_132, .-sovereign_apic_isr_132

.global sovereign_apic_isr_133
.type sovereign_apic_isr_133, @function
sovereign_apic_isr_133:
    push 0
    push 133
    jmp sovereign_apic_common
.size sovereign_apic_isr_133, .-sovereign_apic_isr_133

.global sovereign_apic_isr_134
.type sovereign_apic_isr_134, @function
sovereign_apic_isr_134:
    push 0
    push 134
    jmp sovereign_apic_common
.size sovereign_apic_isr_134, .-sovereign_apic_isr_134

.global sovereign_apic_isr_135
.type sovereign_apic_isr_135, @function
sovereign_apic_isr_135:
    push 0
    push 135
    jmp sovereign_apic_common
.size sovereign_apic_isr_135, .-sovereign_apic_isr_135

.global sovereign_apic_isr_136
.type sovereign_apic_isr_136, @function
sovereign_apic_isr_136:
    push 0
    push 136
    jmp sovereign_apic_common
.size sovereign_apic_isr_136, .-sovereign_apic_isr_136

.global sovereign_apic_isr_137
.type sovereign_apic_isr_137, @function
sovereign_apic_isr_137:
    push 0
    push 137
    jmp sovereign_apic_common
.size sovereign_apic_isr_137, .-sovereign_apic_isr_137

.global sovereign_apic_isr_138
.type sovereign_apic_isr_138, @function
sovereign_apic_isr_138:
    push 0
    push 138
    jmp sovereign_apic_common
.size sovereign_apic_isr_138, .-sovereign_apic_isr_138

.global sovereign_apic_isr_139
.type sovereign_apic_isr_139, @function
sovereign_apic_isr_139:
    push 0
    push 139
    jmp sovereign_apic_common
.size sovereign_apic_isr_139, .-sovereign_apic_isr_139

.global sovereign_apic_isr_140
.type sovereign_apic_isr_140, @function
sovereign_apic_isr_140:
    push 0
    push 140
    jmp sovereign_apic_common
.size sovereign_apic_isr_140, .-sovereign_apic_isr_140

.global sovereign_apic_isr_141
.type sovereign_apic_isr_141, @function
sovereign_apic_isr_141:
    push 0
    push 141
    jmp sovereign_apic_common
.size sovereign_apic_isr_141, .-sovereign_apic_isr_141

.global sovereign_apic_isr_142
.type sovereign_apic_isr_142, @function
sovereign_apic_isr_142:
    push 0
    push 142
    jmp sovereign_apic_common
.size sovereign_apic_isr_142, .-sovereign_apic_isr_142

.global sovereign_apic_isr_143
.type sovereign_apic_isr_143, @function
sovereign_apic_isr_143:
    push 0
    push 143
    jmp sovereign_apic_common
.size sovereign_apic_isr_143, .-sovereign_apic_isr_143

.global sovereign_apic_isr_144
.type sovereign_apic_isr_144, @function
sovereign_apic_isr_144:
    push 0
    push 144
    jmp sovereign_apic_common
.size sovereign_apic_isr_144, .-sovereign_apic_isr_144

.global sovereign_apic_isr_145
.type sovereign_apic_isr_145, @function
sovereign_apic_isr_145:
    push 0
    push 145
    jmp sovereign_apic_common
.size sovereign_apic_isr_145, .-sovereign_apic_isr_145

.global sovereign_apic_isr_146
.type sovereign_apic_isr_146, @function
sovereign_apic_isr_146:
    push 0
    push 146
    jmp sovereign_apic_common
.size sovereign_apic_isr_146, .-sovereign_apic_isr_146

.global sovereign_apic_isr_147
.type sovereign_apic_isr_147, @function
sovereign_apic_isr_147:
    push 0
    push 147
    jmp sovereign_apic_common
.size sovereign_apic_isr_147, .-sovereign_apic_isr_147

.global sovereign_apic_isr_148
.type sovereign_apic_isr_148, @function
sovereign_apic_isr_148:
    push 0
    push 148
    jmp sovereign_apic_common
.size sovereign_apic_isr_148, .-sovereign_apic_isr_148

.global sovereign_apic_isr_149
.type sovereign_apic_isr_149, @function
sovereign_apic_isr_149:
    push 0
    push 149
    jmp sovereign_apic_common
.size sovereign_apic_isr_149, .-sovereign_apic_isr_149

.global sovereign_apic_isr_150
.type sovereign_apic_isr_150, @function
sovereign_apic_isr_150:
    push 0
    push 150
    jmp sovereign_apic_common
.size sovereign_apic_isr_150, .-sovereign_apic_isr_150

.global sovereign_apic_isr_151
.type sovereign_apic_isr_151, @function
sovereign_apic_isr_151:
    push 0
    push 151
    jmp sovereign_apic_common
.size sovereign_apic_isr_151, .-sovereign_apic_isr_151

.global sovereign_apic_isr_152
.type sovereign_apic_isr_152, @function
sovereign_apic_isr_152:
    push 0
    push 152
    jmp sovereign_apic_common
.size sovereign_apic_isr_152, .-sovereign_apic_isr_152

.global sovereign_apic_isr_153
.type sovereign_apic_isr_153, @function
sovereign_apic_isr_153:
    push 0
    push 153
    jmp sovereign_apic_common
.size sovereign_apic_isr_153, .-sovereign_apic_isr_153

.global sovereign_apic_isr_154
.type sovereign_apic_isr_154, @function
sovereign_apic_isr_154:
    push 0
    push 154
    jmp sovereign_apic_common
.size sovereign_apic_isr_154, .-sovereign_apic_isr_154

.global sovereign_apic_isr_155
.type sovereign_apic_isr_155, @function
sovereign_apic_isr_155:
    push 0
    push 155
    jmp sovereign_apic_common
.size sovereign_apic_isr_155, .-sovereign_apic_isr_155

.global sovereign_apic_isr_156
.type sovereign_apic_isr_156, @function
sovereign_apic_isr_156:
    push 0
    push 156
    jmp sovereign_apic_common
.size sovereign_apic_isr_156, .-sovereign_apic_isr_156

.global sovereign_apic_isr_157
.type sovereign_apic_isr_157, @function
sovereign_apic_isr_157:
    push 0
    push 157
    jmp sovereign_apic_common
.size sovereign_apic_isr_157, .-sovereign_apic_isr_157

.global sovereign_apic_isr_158
.type sovereign_apic_isr_158, @function
sovereign_apic_isr_158:
    push 0
    push 158
    jmp sovereign_apic_common
.size sovereign_apic_isr_158, .-sovereign_apic_isr_158

.global sovereign_apic_isr_159
.type sovereign_apic_isr_159, @function
sovereign_apic_isr_159:
    push 0
    push 159
    jmp sovereign_apic_common
.size sovereign_apic_isr_159, .-sovereign_apic_isr_159

.global sovereign_apic_isr_160
.type sovereign_apic_isr_160, @function
sovereign_apic_isr_160:
    push 0
    push 160
    jmp sovereign_apic_common
.size sovereign_apic_isr_160, .-sovereign_apic_isr_160

.global sovereign_apic_isr_161
.type sovereign_apic_isr_161, @function
sovereign_apic_isr_161:
    push 0
    push 161
    jmp sovereign_apic_common
.size sovereign_apic_isr_161, .-sovereign_apic_isr_161

.global sovereign_apic_isr_162
.type sovereign_apic_isr_162, @function
sovereign_apic_isr_162:
    push 0
    push 162
    jmp sovereign_apic_common
.size sovereign_apic_isr_162, .-sovereign_apic_isr_162

.global sovereign_apic_isr_163
.type sovereign_apic_isr_163, @function
sovereign_apic_isr_163:
    push 0
    push 163
    jmp sovereign_apic_common
.size sovereign_apic_isr_163, .-sovereign_apic_isr_163

.global sovereign_apic_isr_164
.type sovereign_apic_isr_164, @function
sovereign_apic_isr_164:
    push 0
    push 164
    jmp sovereign_apic_common
.size sovereign_apic_isr_164, .-sovereign_apic_isr_164

.global sovereign_apic_isr_165
.type sovereign_apic_isr_165, @function
sovereign_apic_isr_165:
    push 0
    push 165
    jmp sovereign_apic_common
.size sovereign_apic_isr_165, .-sovereign_apic_isr_165

.global sovereign_apic_isr_166
.type sovereign_apic_isr_166, @function
sovereign_apic_isr_166:
    push 0
    push 166
    jmp sovereign_apic_common
.size sovereign_apic_isr_166, .-sovereign_apic_isr_166

.global sovereign_apic_isr_167
.type sovereign_apic_isr_167, @function
sovereign_apic_isr_167:
    push 0
    push 167
    jmp sovereign_apic_common
.size sovereign_apic_isr_167, .-sovereign_apic_isr_167

.global sovereign_apic_isr_168
.type sovereign_apic_isr_168, @function
sovereign_apic_isr_168:
    push 0
    push 168
    jmp sovereign_apic_common
.size sovereign_apic_isr_168, .-sovereign_apic_isr_168

.global sovereign_apic_isr_169
.type sovereign_apic_isr_169, @function
sovereign_apic_isr_169:
    push 0
    push 169
    jmp sovereign_apic_common
.size sovereign_apic_isr_169, .-sovereign_apic_isr_169

.global sovereign_apic_isr_170
.type sovereign_apic_isr_170, @function
sovereign_apic_isr_170:
    push 0
    push 170
    jmp sovereign_apic_common
.size sovereign_apic_isr_170, .-sovereign_apic_isr_170

.global sovereign_apic_isr_171
.type sovereign_apic_isr_171, @function
sovereign_apic_isr_171:
    push 0
    push 171
    jmp sovereign_apic_common
.size sovereign_apic_isr_171, .-sovereign_apic_isr_171

.global sovereign_apic_isr_172
.type sovereign_apic_isr_172, @function
sovereign_apic_isr_172:
    push 0
    push 172
    jmp sovereign_apic_common
.size sovereign_apic_isr_172, .-sovereign_apic_isr_172

.global sovereign_apic_isr_173
.type sovereign_apic_isr_173, @function
sovereign_apic_isr_173:
    push 0
    push 173
    jmp sovereign_apic_common
.size sovereign_apic_isr_173, .-sovereign_apic_isr_173

.global sovereign_apic_isr_174
.type sovereign_apic_isr_174, @function
sovereign_apic_isr_174:
    push 0
    push 174
    jmp sovereign_apic_common
.size sovereign_apic_isr_174, .-sovereign_apic_isr_174

.global sovereign_apic_isr_175
.type sovereign_apic_isr_175, @function
sovereign_apic_isr_175:
    push 0
    push 175
    jmp sovereign_apic_common
.size sovereign_apic_isr_175, .-sovereign_apic_isr_175

.global sovereign_apic_isr_176
.type sovereign_apic_isr_176, @function
sovereign_apic_isr_176:
    push 0
    push 176
    jmp sovereign_apic_common
.size sovereign_apic_isr_176, .-sovereign_apic_isr_176

.global sovereign_apic_isr_177
.type sovereign_apic_isr_177, @function
sovereign_apic_isr_177:
    push 0
    push 177
    jmp sovereign_apic_common
.size sovereign_apic_isr_177, .-sovereign_apic_isr_177

.global sovereign_apic_isr_178
.type sovereign_apic_isr_178, @function
sovereign_apic_isr_178:
    push 0
    push 178
    jmp sovereign_apic_common
.size sovereign_apic_isr_178, .-sovereign_apic_isr_178

.global sovereign_apic_isr_179
.type sovereign_apic_isr_179, @function
sovereign_apic_isr_179:
    push 0
    push 179
    jmp sovereign_apic_common
.size sovereign_apic_isr_179, .-sovereign_apic_isr_179

.global sovereign_apic_isr_180
.type sovereign_apic_isr_180, @function
sovereign_apic_isr_180:
    push 0
    push 180
    jmp sovereign_apic_common
.size sovereign_apic_isr_180, .-sovereign_apic_isr_180

.global sovereign_apic_isr_181
.type sovereign_apic_isr_181, @function
sovereign_apic_isr_181:
    push 0
    push 181
    jmp sovereign_apic_common
.size sovereign_apic_isr_181, .-sovereign_apic_isr_181

.global sovereign_apic_isr_182
.type sovereign_apic_isr_182, @function
sovereign_apic_isr_182:
    push 0
    push 182
    jmp sovereign_apic_common
.size sovereign_apic_isr_182, .-sovereign_apic_isr_182

.global sovereign_apic_isr_183
.type sovereign_apic_isr_183, @function
sovereign_apic_isr_183:
    push 0
    push 183
    jmp sovereign_apic_common
.size sovereign_apic_isr_183, .-sovereign_apic_isr_183

.global sovereign_apic_isr_184
.type sovereign_apic_isr_184, @function
sovereign_apic_isr_184:
    push 0
    push 184
    jmp sovereign_apic_common
.size sovereign_apic_isr_184, .-sovereign_apic_isr_184

.global sovereign_apic_isr_185
.type sovereign_apic_isr_185, @function
sovereign_apic_isr_185:
    push 0
    push 185
    jmp sovereign_apic_common
.size sovereign_apic_isr_185, .-sovereign_apic_isr_185

.global sovereign_apic_isr_186
.type sovereign_apic_isr_186, @function
sovereign_apic_isr_186:
    push 0
    push 186
    jmp sovereign_apic_common
.size sovereign_apic_isr_186, .-sovereign_apic_isr_186

.global sovereign_apic_isr_187
.type sovereign_apic_isr_187, @function
sovereign_apic_isr_187:
    push 0
    push 187
    jmp sovereign_apic_common
.size sovereign_apic_isr_187, .-sovereign_apic_isr_187

.global sovereign_apic_isr_188
.type sovereign_apic_isr_188, @function
sovereign_apic_isr_188:
    push 0
    push 188
    jmp sovereign_apic_common
.size sovereign_apic_isr_188, .-sovereign_apic_isr_188

.global sovereign_apic_isr_189
.type sovereign_apic_isr_189, @function
sovereign_apic_isr_189:
    push 0
    push 189
    jmp sovereign_apic_common
.size sovereign_apic_isr_189, .-sovereign_apic_isr_189

.global sovereign_apic_isr_190
.type sovereign_apic_isr_190, @function
sovereign_apic_isr_190:
    push 0
    push 190
    jmp sovereign_apic_common
.size sovereign_apic_isr_190, .-sovereign_apic_isr_190

.global sovereign_apic_isr_191
.type sovereign_apic_isr_191, @function
sovereign_apic_isr_191:
    push 0
    push 191
    jmp sovereign_apic_common
.size sovereign_apic_isr_191, .-sovereign_apic_isr_191

.global sovereign_apic_isr_192
.type sovereign_apic_isr_192, @function
sovereign_apic_isr_192:
    push 0
    push 192
    jmp sovereign_apic_common
.size sovereign_apic_isr_192, .-sovereign_apic_isr_192

.global sovereign_apic_isr_193
.type sovereign_apic_isr_193, @function
sovereign_apic_isr_193:
    push 0
    push 193
    jmp sovereign_apic_common
.size sovereign_apic_isr_193, .-sovereign_apic_isr_193

.global sovereign_apic_isr_194
.type sovereign_apic_isr_194, @function
sovereign_apic_isr_194:
    push 0
    push 194
    jmp sovereign_apic_common
.size sovereign_apic_isr_194, .-sovereign_apic_isr_194

.global sovereign_apic_isr_195
.type sovereign_apic_isr_195, @function
sovereign_apic_isr_195:
    push 0
    push 195
    jmp sovereign_apic_common
.size sovereign_apic_isr_195, .-sovereign_apic_isr_195

.global sovereign_apic_isr_196
.type sovereign_apic_isr_196, @function
sovereign_apic_isr_196:
    push 0
    push 196
    jmp sovereign_apic_common
.size sovereign_apic_isr_196, .-sovereign_apic_isr_196

.global sovereign_apic_isr_197
.type sovereign_apic_isr_197, @function
sovereign_apic_isr_197:
    push 0
    push 197
    jmp sovereign_apic_common
.size sovereign_apic_isr_197, .-sovereign_apic_isr_197

.global sovereign_apic_isr_198
.type sovereign_apic_isr_198, @function
sovereign_apic_isr_198:
    push 0
    push 198
    jmp sovereign_apic_common
.size sovereign_apic_isr_198, .-sovereign_apic_isr_198

.global sovereign_apic_isr_199
.type sovereign_apic_isr_199, @function
sovereign_apic_isr_199:
    push 0
    push 199
    jmp sovereign_apic_common
.size sovereign_apic_isr_199, .-sovereign_apic_isr_199

.global sovereign_apic_isr_200
.type sovereign_apic_isr_200, @function
sovereign_apic_isr_200:
    push 0
    push 200
    jmp sovereign_apic_common
.size sovereign_apic_isr_200, .-sovereign_apic_isr_200

.global sovereign_apic_isr_201
.type sovereign_apic_isr_201, @function
sovereign_apic_isr_201:
    push 0
    push 201
    jmp sovereign_apic_common
.size sovereign_apic_isr_201, .-sovereign_apic_isr_201

.global sovereign_apic_isr_202
.type sovereign_apic_isr_202, @function
sovereign_apic_isr_202:
    push 0
    push 202
    jmp sovereign_apic_common
.size sovereign_apic_isr_202, .-sovereign_apic_isr_202

.global sovereign_apic_isr_203
.type sovereign_apic_isr_203, @function
sovereign_apic_isr_203:
    push 0
    push 203
    jmp sovereign_apic_common
.size sovereign_apic_isr_203, .-sovereign_apic_isr_203

.global sovereign_apic_isr_204
.type sovereign_apic_isr_204, @function
sovereign_apic_isr_204:
    push 0
    push 204
    jmp sovereign_apic_common
.size sovereign_apic_isr_204, .-sovereign_apic_isr_204

.global sovereign_apic_isr_205
.type sovereign_apic_isr_205, @function
sovereign_apic_isr_205:
    push 0
    push 205
    jmp sovereign_apic_common
.size sovereign_apic_isr_205, .-sovereign_apic_isr_205

.global sovereign_apic_isr_206
.type sovereign_apic_isr_206, @function
sovereign_apic_isr_206:
    push 0
    push 206
    jmp sovereign_apic_common
.size sovereign_apic_isr_206, .-sovereign_apic_isr_206

.global sovereign_apic_isr_207
.type sovereign_apic_isr_207, @function
sovereign_apic_isr_207:
    push 0
    push 207
    jmp sovereign_apic_common
.size sovereign_apic_isr_207, .-sovereign_apic_isr_207

.global sovereign_apic_isr_208
.type sovereign_apic_isr_208, @function
sovereign_apic_isr_208:
    push 0
    push 208
    jmp sovereign_apic_common
.size sovereign_apic_isr_208, .-sovereign_apic_isr_208

.global sovereign_apic_isr_209
.type sovereign_apic_isr_209, @function
sovereign_apic_isr_209:
    push 0
    push 209
    jmp sovereign_apic_common
.size sovereign_apic_isr_209, .-sovereign_apic_isr_209

.global sovereign_apic_isr_210
.type sovereign_apic_isr_210, @function
sovereign_apic_isr_210:
    push 0
    push 210
    jmp sovereign_apic_common
.size sovereign_apic_isr_210, .-sovereign_apic_isr_210

.global sovereign_apic_isr_211
.type sovereign_apic_isr_211, @function
sovereign_apic_isr_211:
    push 0
    push 211
    jmp sovereign_apic_common
.size sovereign_apic_isr_211, .-sovereign_apic_isr_211

.global sovereign_apic_isr_212
.type sovereign_apic_isr_212, @function
sovereign_apic_isr_212:
    push 0
    push 212
    jmp sovereign_apic_common
.size sovereign_apic_isr_212, .-sovereign_apic_isr_212

.global sovereign_apic_isr_213
.type sovereign_apic_isr_213, @function
sovereign_apic_isr_213:
    push 0
    push 213
    jmp sovereign_apic_common
.size sovereign_apic_isr_213, .-sovereign_apic_isr_213

.global sovereign_apic_isr_214
.type sovereign_apic_isr_214, @function
sovereign_apic_isr_214:
    push 0
    push 214
    jmp sovereign_apic_common
.size sovereign_apic_isr_214, .-sovereign_apic_isr_214

.global sovereign_apic_isr_215
.type sovereign_apic_isr_215, @function
sovereign_apic_isr_215:
    push 0
    push 215
    jmp sovereign_apic_common
.size sovereign_apic_isr_215, .-sovereign_apic_isr_215

.global sovereign_apic_isr_216
.type sovereign_apic_isr_216, @function
sovereign_apic_isr_216:
    push 0
    push 216
    jmp sovereign_apic_common
.size sovereign_apic_isr_216, .-sovereign_apic_isr_216

.global sovereign_apic_isr_217
.type sovereign_apic_isr_217, @function
sovereign_apic_isr_217:
    push 0
    push 217
    jmp sovereign_apic_common
.size sovereign_apic_isr_217, .-sovereign_apic_isr_217

.global sovereign_apic_isr_218
.type sovereign_apic_isr_218, @function
sovereign_apic_isr_218:
    push 0
    push 218
    jmp sovereign_apic_common
.size sovereign_apic_isr_218, .-sovereign_apic_isr_218

.global sovereign_apic_isr_219
.type sovereign_apic_isr_219, @function
sovereign_apic_isr_219:
    push 0
    push 219
    jmp sovereign_apic_common
.size sovereign_apic_isr_219, .-sovereign_apic_isr_219

.global sovereign_apic_isr_220
.type sovereign_apic_isr_220, @function
sovereign_apic_isr_220:
    push 0
    push 220
    jmp sovereign_apic_common
.size sovereign_apic_isr_220, .-sovereign_apic_isr_220

.global sovereign_apic_isr_221
.type sovereign_apic_isr_221, @function
sovereign_apic_isr_221:
    push 0
    push 221
    jmp sovereign_apic_common
.size sovereign_apic_isr_221, .-sovereign_apic_isr_221

.global sovereign_apic_isr_222
.type sovereign_apic_isr_222, @function
sovereign_apic_isr_222:
    push 0
    push 222
    jmp sovereign_apic_common
.size sovereign_apic_isr_222, .-sovereign_apic_isr_222

.global sovereign_apic_isr_223
.type sovereign_apic_isr_223, @function
sovereign_apic_isr_223:
    push 0
    push 223
    jmp sovereign_apic_common
.size sovereign_apic_isr_223, .-sovereign_apic_isr_223

.global sovereign_apic_isr_224
.type sovereign_apic_isr_224, @function
sovereign_apic_isr_224:
    push 0
    push 224
    jmp sovereign_apic_common
.size sovereign_apic_isr_224, .-sovereign_apic_isr_224

.global sovereign_apic_isr_225
.type sovereign_apic_isr_225, @function
sovereign_apic_isr_225:
    push 0
    push 225
    jmp sovereign_apic_common
.size sovereign_apic_isr_225, .-sovereign_apic_isr_225

.global sovereign_apic_isr_226
.type sovereign_apic_isr_226, @function
sovereign_apic_isr_226:
    push 0
    push 226
    jmp sovereign_apic_common
.size sovereign_apic_isr_226, .-sovereign_apic_isr_226

.global sovereign_apic_isr_227
.type sovereign_apic_isr_227, @function
sovereign_apic_isr_227:
    push 0
    push 227
    jmp sovereign_apic_common
.size sovereign_apic_isr_227, .-sovereign_apic_isr_227

.global sovereign_apic_isr_228
.type sovereign_apic_isr_228, @function
sovereign_apic_isr_228:
    push 0
    push 228
    jmp sovereign_apic_common
.size sovereign_apic_isr_228, .-sovereign_apic_isr_228

.global sovereign_apic_isr_229
.type sovereign_apic_isr_229, @function
sovereign_apic_isr_229:
    push 0
    push 229
    jmp sovereign_apic_common
.size sovereign_apic_isr_229, .-sovereign_apic_isr_229

.global sovereign_apic_isr_230
.type sovereign_apic_isr_230, @function
sovereign_apic_isr_230:
    push 0
    push 230
    jmp sovereign_apic_common
.size sovereign_apic_isr_230, .-sovereign_apic_isr_230

.global sovereign_apic_isr_231
.type sovereign_apic_isr_231, @function
sovereign_apic_isr_231:
    push 0
    push 231
    jmp sovereign_apic_common
.size sovereign_apic_isr_231, .-sovereign_apic_isr_231

.global sovereign_apic_isr_232
.type sovereign_apic_isr_232, @function
sovereign_apic_isr_232:
    push 0
    push 232
    jmp sovereign_apic_common
.size sovereign_apic_isr_232, .-sovereign_apic_isr_232

.global sovereign_apic_isr_233
.type sovereign_apic_isr_233, @function
sovereign_apic_isr_233:
    push 0
    push 233
    jmp sovereign_apic_common
.size sovereign_apic_isr_233, .-sovereign_apic_isr_233

.global sovereign_apic_isr_234
.type sovereign_apic_isr_234, @function
sovereign_apic_isr_234:
    push 0
    push 234
    jmp sovereign_apic_common
.size sovereign_apic_isr_234, .-sovereign_apic_isr_234

.global sovereign_apic_isr_235
.type sovereign_apic_isr_235, @function
sovereign_apic_isr_235:
    push 0
    push 235
    jmp sovereign_apic_common
.size sovereign_apic_isr_235, .-sovereign_apic_isr_235

.global sovereign_apic_isr_236
.type sovereign_apic_isr_236, @function
sovereign_apic_isr_236:
    push 0
    push 236
    jmp sovereign_apic_common
.size sovereign_apic_isr_236, .-sovereign_apic_isr_236

.global sovereign_apic_isr_237
.type sovereign_apic_isr_237, @function
sovereign_apic_isr_237:
    push 0
    push 237
    jmp sovereign_apic_common
.size sovereign_apic_isr_237, .-sovereign_apic_isr_237

.global sovereign_apic_isr_238
.type sovereign_apic_isr_238, @function
sovereign_apic_isr_238:
    push 0
    push 238
    jmp sovereign_apic_common
.size sovereign_apic_isr_238, .-sovereign_apic_isr_238

.global sovereign_apic_isr_239
.type sovereign_apic_isr_239, @function
sovereign_apic_isr_239:
    push 0
    push 239
    jmp sovereign_apic_common
.size sovereign_apic_isr_239, .-sovereign_apic_isr_239

.global sovereign_apic_isr_240
.type sovereign_apic_isr_240, @function
sovereign_apic_isr_240:
    push 0
    push 240
    jmp sovereign_apic_common
.size sovereign_apic_isr_240, .-sovereign_apic_isr_240

.global sovereign_apic_isr_241
.type sovereign_apic_isr_241, @function
sovereign_apic_isr_241:
    push 0
    push 241
    jmp sovereign_apic_common
.size sovereign_apic_isr_241, .-sovereign_apic_isr_241

.global sovereign_apic_isr_242
.type sovereign_apic_isr_242, @function
sovereign_apic_isr_242:
    push 0
    push 242
    jmp sovereign_apic_common
.size sovereign_apic_isr_242, .-sovereign_apic_isr_242

.global sovereign_apic_isr_243
.type sovereign_apic_isr_243, @function
sovereign_apic_isr_243:
    push 0
    push 243
    jmp sovereign_apic_common
.size sovereign_apic_isr_243, .-sovereign_apic_isr_243

.global sovereign_apic_isr_244
.type sovereign_apic_isr_244, @function
sovereign_apic_isr_244:
    push 0
    push 244
    jmp sovereign_apic_common
.size sovereign_apic_isr_244, .-sovereign_apic_isr_244

.global sovereign_apic_isr_245
.type sovereign_apic_isr_245, @function
sovereign_apic_isr_245:
    push 0
    push 245
    jmp sovereign_apic_common
.size sovereign_apic_isr_245, .-sovereign_apic_isr_245

.global sovereign_apic_isr_246
.type sovereign_apic_isr_246, @function
sovereign_apic_isr_246:
    push 0
    push 246
    jmp sovereign_apic_common
.size sovereign_apic_isr_246, .-sovereign_apic_isr_246

.global sovereign_apic_isr_247
.type sovereign_apic_isr_247, @function
sovereign_apic_isr_247:
    push 0
    push 247
    jmp sovereign_apic_common
.size sovereign_apic_isr_247, .-sovereign_apic_isr_247

.global sovereign_apic_isr_248
.type sovereign_apic_isr_248, @function
sovereign_apic_isr_248:
    push 0
    push 248
    jmp sovereign_apic_common
.size sovereign_apic_isr_248, .-sovereign_apic_isr_248

.global sovereign_apic_isr_249
.type sovereign_apic_isr_249, @function
sovereign_apic_isr_249:
    push 0
    push 249
    jmp sovereign_apic_common
.size sovereign_apic_isr_249, .-sovereign_apic_isr_249

.global sovereign_apic_isr_250
.type sovereign_apic_isr_250, @function
sovereign_apic_isr_250:
    push 0
    push 250
    jmp sovereign_apic_common
.size sovereign_apic_isr_250, .-sovereign_apic_isr_250

.global sovereign_apic_isr_251
.type sovereign_apic_isr_251, @function
sovereign_apic_isr_251:
    push 0
    push 251
    jmp sovereign_apic_common
.size sovereign_apic_isr_251, .-sovereign_apic_isr_251

.global sovereign_apic_isr_252
.type sovereign_apic_isr_252, @function
sovereign_apic_isr_252:
    push 0
    push 252
    jmp sovereign_apic_common
.size sovereign_apic_isr_252, .-sovereign_apic_isr_252

.global sovereign_apic_isr_253
.type sovereign_apic_isr_253, @function
sovereign_apic_isr_253:
    push 0
    push 253
    jmp sovereign_apic_common
.size sovereign_apic_isr_253, .-sovereign_apic_isr_253

.global sovereign_apic_isr_254
.type sovereign_apic_isr_254, @function
sovereign_apic_isr_254:
    push 0
    push 254
    jmp sovereign_apic_common
.size sovereign_apic_isr_254, .-sovereign_apic_isr_254

.global sovereign_apic_stub_table
.type sovereign_apic_stub_table, @object
.align 8
sovereign_apic_stub_table:
.quad sovereign_apic_isr_48, .quad sovereign_apic_isr_49, .quad sovereign_apic_isr_50, .quad sovereign_apic_isr_51
.quad sovereign_apic_isr_52, .quad sovereign_apic_isr_53, .quad sovereign_apic_isr_54, .quad sovereign_apic_isr_55
.quad sovereign_apic_isr_56, .quad sovereign_apic_isr_57, .quad sovereign_apic_isr_58, .quad sovereign_apic_isr_59
.quad sovereign_apic_isr_60, .quad sovereign_apic_isr_61, .quad sovereign_apic_isr_62, .quad sovereign_apic_isr_63
.quad sovereign_apic_isr_64, .quad sovereign_apic_isr_65, .quad sovereign_apic_isr_66, .quad sovereign_apic_isr_67
.quad sovereign_apic_isr_68, .quad sovereign_apic_isr_69, .quad sovereign_apic_isr_70, .quad sovereign_apic_isr_71
.quad sovereign_apic_isr_72, .quad sovereign_apic_isr_73, .quad sovereign_apic_isr_74, .quad sovereign_apic_isr_75
.quad sovereign_apic_isr_76, .quad sovereign_apic_isr_77, .quad sovereign_apic_isr_78, .quad sovereign_apic_isr_79
.quad sovereign_apic_isr_80, .quad sovereign_apic_isr_81, .quad sovereign_apic_isr_82, .quad sovereign_apic_isr_83
.quad sovereign_apic_isr_84, .quad sovereign_apic_isr_85, .quad sovereign_apic_isr_86, .quad sovereign_apic_isr_87
.quad sovereign_apic_isr_88, .quad sovereign_apic_isr_89, .quad sovereign_apic_isr_90, .quad sovereign_apic_isr_91
.quad sovereign_apic_isr_92, .quad sovereign_apic_isr_93, .quad sovereign_apic_isr_94, .quad sovereign_apic_isr_95
.quad sovereign_apic_isr_96, .quad sovereign_apic_isr_97, .quad sovereign_apic_isr_98, .quad sovereign_apic_isr_99
.quad sovereign_apic_isr_100, .quad sovereign_apic_isr_101, .quad sovereign_apic_isr_102, .quad sovereign_apic_isr_103
.quad sovereign_apic_isr_104, .quad sovereign_apic_isr_105, .quad sovereign_apic_isr_106, .quad sovereign_apic_isr_107
.quad sovereign_apic_isr_108, .quad sovereign_apic_isr_109, .quad sovereign_apic_isr_110, .quad sovereign_apic_isr_111
.quad sovereign_apic_isr_112, .quad sovereign_apic_isr_113, .quad sovereign_apic_isr_114, .quad sovereign_apic_isr_115
.quad sovereign_apic_isr_116, .quad sovereign_apic_isr_117, .quad sovereign_apic_isr_118, .quad sovereign_apic_isr_119
.quad sovereign_apic_isr_120, .quad sovereign_apic_isr_121, .quad sovereign_apic_isr_122, .quad sovereign_apic_isr_123
.quad sovereign_apic_isr_124, .quad sovereign_apic_isr_125, .quad sovereign_apic_isr_126, .quad sovereign_apic_isr_127
.quad sovereign_apic_isr_129, .quad sovereign_apic_isr_130, .quad sovereign_apic_isr_131, .quad sovereign_apic_isr_132
.quad sovereign_apic_isr_133, .quad sovereign_apic_isr_134, .quad sovereign_apic_isr_135, .quad sovereign_apic_isr_136
.quad sovereign_apic_isr_137, .quad sovereign_apic_isr_138, .quad sovereign_apic_isr_139, .quad sovereign_apic_isr_140
.quad sovereign_apic_isr_141, .quad sovereign_apic_isr_142, .quad sovereign_apic_isr_143, .quad sovereign_apic_isr_144
.quad sovereign_apic_isr_145, .quad sovereign_apic_isr_146, .quad sovereign_apic_isr_147, .quad sovereign_apic_isr_148
.quad sovereign_apic_isr_149, .quad sovereign_apic_isr_150, .quad sovereign_apic_isr_151, .quad sovereign_apic_isr_152
.quad sovereign_apic_isr_153, .quad sovereign_apic_isr_154, .quad sovereign_apic_isr_155, .quad sovereign_apic_isr_156
.quad sovereign_apic_isr_157, .quad sovereign_apic_isr_158, .quad sovereign_apic_isr_159, .quad sovereign_apic_isr_160
.quad sovereign_apic_isr_161, .quad sovereign_apic_isr_162, .quad sovereign_apic_isr_163, .quad sovereign_apic_isr_164
.quad sovereign_apic_isr_165, .quad sovereign_apic_isr_166, .quad sovereign_apic_isr_167, .quad sovereign_apic_isr_168
.quad sovereign_apic_isr_169, .quad sovereign_apic_isr_170, .quad sovereign_apic_isr_171, .quad sovereign_apic_isr_172
.quad sovereign_apic_isr_173, .quad sovereign_apic_isr_174, .quad sovereign_apic_isr_175, .quad sovereign_apic_isr_176
.quad sovereign_apic_isr_177, .quad sovereign_apic_isr_178, .quad sovereign_apic_isr_179, .quad sovereign_apic_isr_180
.quad sovereign_apic_isr_181, .quad sovereign_apic_isr_182, .quad sovereign_apic_isr_183, .quad sovereign_apic_isr_184
.quad sovereign_apic_isr_185, .quad sovereign_apic_isr_186, .quad sovereign_apic_isr_187, .quad sovereign_apic_isr_188
.quad sovereign_apic_isr_189, .quad sovereign_apic_isr_190, .quad sovereign_apic_isr_191, .quad sovereign_apic_isr_192
.quad sovereign_apic_isr_193, .quad sovereign_apic_isr_194, .quad sovereign_apic_isr_195, .quad sovereign_apic_isr_196
.quad sovereign_apic_isr_197, .quad sovereign_apic_isr_198, .quad sovereign_apic_isr_199, .quad sovereign_apic_isr_200
.quad sovereign_apic_isr_201, .quad sovereign_apic_isr_202, .quad sovereign_apic_isr_203, .quad sovereign_apic_isr_204
.quad sovereign_apic_isr_205, .quad sovereign_apic_isr_206, .quad sovereign_apic_isr_207, .quad sovereign_apic_isr_208
.quad sovereign_apic_isr_209, .quad sovereign_apic_isr_210, .quad sovereign_apic_isr_211, .quad sovereign_apic_isr_212
.quad sovereign_apic_isr_213, .quad sovereign_apic_isr_214, .quad sovereign_apic_isr_215, .quad sovereign_apic_isr_216
.quad sovereign_apic_isr_217, .quad sovereign_apic_isr_218, .quad sovereign_apic_isr_219, .quad sovereign_apic_isr_220
.quad sovereign_apic_isr_221, .quad sovereign_apic_isr_222, .quad sovereign_apic_isr_223, .quad sovereign_apic_isr_224
.quad sovereign_apic_isr_225, .quad sovereign_apic_isr_226, .quad sovereign_apic_isr_227, .quad sovereign_apic_isr_228
.quad sovereign_apic_isr_229, .quad sovereign_apic_isr_230, .quad sovereign_apic_isr_231, .quad sovereign_apic_isr_232
.quad sovereign_apic_isr_233, .quad sovereign_apic_isr_234, .quad sovereign_apic_isr_235, .quad sovereign_apic_isr_236
.quad sovereign_apic_isr_237, .quad sovereign_apic_isr_238, .quad sovereign_apic_isr_239, .quad sovereign_apic_isr_240
.quad sovereign_apic_isr_241, .quad sovereign_apic_isr_242, .quad sovereign_apic_isr_243, .quad sovereign_apic_isr_244
.quad sovereign_apic_isr_245, .quad sovereign_apic_isr_246, .quad sovereign_apic_isr_247, .quad sovereign_apic_isr_248
.quad sovereign_apic_isr_249, .quad sovereign_apic_isr_250, .quad sovereign_apic_isr_251, .quad sovereign_apic_isr_252
.quad sovereign_apic_isr_253, .quad sovereign_apic_isr_254
.size sovereign_apic_stub_table, .-sovereign_apic_stub_table

.global sovereign_apic_common
.type sovereign_apic_common, @function
sovereign_apic_common:
    # Preserve every GPR; the Rust frame layout matches ExceptionFrame.
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

    # Keep the original frame while aligning the call stack and saving xstate.
    cld
    mov r12, rsp
    mov rdi, rsp
    and rsp, -16
    sub rsp, 512
    fxsave64 [rsp]
    call sovereign_apic_dispatch
    fxrstor64 [rsp]
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
.size sovereign_apic_common, .-sovereign_apic_common
