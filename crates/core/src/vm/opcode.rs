//! BGI / Buriko 字节码操作码定义 (OpCode)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpCode(pub u32);

impl OpCode {
    pub const PUSH_ZERO: OpCode = OpCode(0x0000);
    pub const PUSH_INT: OpCode = OpCode(0x0001);
    pub const PUSH_ADDR: OpCode = OpCode(0x0001);
    pub const PUSH_CONST: OpCode = OpCode(0x0002);
    pub const PUSH_LOCAL: OpCode = OpCode(0x0002);
    pub const PUSH_STRING: OpCode = OpCode(0x0003);
    pub const STORE: OpCode = OpCode(0x0009);
    pub const LOAD_SP: OpCode = OpCode(0x0010);
    pub const STORE_SP: OpCode = OpCode(0x0011);
    pub const CALL_SYS: OpCode = OpCode(0x0016);
    pub const JMP_LABEL: OpCode = OpCode(0x0017);
    pub const JMP: OpCode = OpCode(0x0018);
    pub const JZ: OpCode = OpCode(0x0019);
    pub const RET: OpCode = OpCode(0x001b);
    pub const CALL_FUNC: OpCode = OpCode(0x001c);
    pub const ADD: OpCode = OpCode(0x0020);
    pub const SUB: OpCode = OpCode(0x0021);
    pub const MUL: OpCode = OpCode(0x0022);
    pub const DIV: OpCode = OpCode(0x0023);
    pub const CMP_EQ: OpCode = OpCode(0x0030);
    pub const CMP_GE: OpCode = OpCode(0x0033);
    pub const LOGICAL_AND: OpCode = OpCode(0x0038);
    pub const DEBUG_LINE: OpCode = OpCode(0x003f);
    pub const STMT_END: OpCode = OpCode(0x003f);
    pub const CALL_SUBSCRIPT: OpCode = OpCode(0x00f0);
    pub const CALL_SCRIPT: OpCode = OpCode(0x00f3);
    pub const END_SCRIPT: OpCode = OpCode(0x00f4);
    pub const SUBSCRIPT_PREP: OpCode = OpCode(0x00f6);
    pub const LABEL_CMP: OpCode = OpCode(0x00f7);
    pub const LABEL_END: OpCode = OpCode(0x00f9);
    pub const DIALOGUE: OpCode = OpCode(0x0140);
    pub const DIALOGUE_ALT: OpCode = OpCode(0x0143);
    pub const CHOICE: OpCode = OpCode(0x0160);

    /// 是否为标准 BGI 8 字节定长指令 (4B opcode + 4B inline arg)
    /// 实证全集: {0x0000, 0x0001, 0x0002, 0x0003, 0x0009, 0x0017, 0x0019}；0x0010 为 4 字节
    #[inline]
    pub fn is_8byte_instruction(&self) -> bool {
        matches!(
            self.0,
            0x0000 | 0x0001 | 0x0002 | 0x0003 | 0x0009 | 0x0017 | 0x0019
        )
    }

    /// 是否为台词/对白渲染指令 (0x140 正常台词 或 0x143 履历专有/无差分隐藏台词)
    #[inline]
    pub fn is_dialogue(&self) -> bool {
        self.0 == 0x0140 || self.0 == 0x0143
    }

    /// 是否为字符串压栈指令 (0x0003)
    #[inline]
    pub fn is_push_string(&self) -> bool {
        self.0 == 0x0003
    }

    /// 是否为用户函数调用指令 (0x001C)
    #[inline]
    pub fn is_call_func(&self) -> bool {
        self.0 == 0x001c
    }

    /// 是否为分支选择项指令 (0x0160)
    #[inline]
    pub fn is_choice(&self) -> bool {
        self.0 == 0x0160
    }

    /// 是否为调试行号指令 (0x003F)
    #[inline]
    pub fn is_debug_line(&self) -> bool {
        self.0 == 0x003f
    }
}

impl std::fmt::Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::PUSH_ZERO => write!(f, "PUSH_ZERO (0x0000)"),
            Self::PUSH_INT => write!(f, "PUSH_INT (0x0001)"),
            Self::PUSH_CONST => write!(f, "PUSH_CONST (0x0002)"),
            Self::PUSH_STRING => write!(f, "PUSH_STRING (0x0003)"),
            Self::CALL_SYS => write!(f, "CALL_SYS (0x0016)"),
            Self::CALL_FUNC => write!(f, "CALL_FUNC (0x001c)"),
            Self::DEBUG_LINE => write!(f, "DEBUG_LINE (0x003f)"),
            Self::DIALOGUE => write!(f, "DIALOGUE (0x0140)"),
            Self::DIALOGUE_ALT => write!(f, "DIALOGUE_ALT (0x0143)"),
            Self::CHOICE => write!(f, "CHOICE (0x0160)"),
            _ => write!(f, "OP_0x{:04x}", self.0),
        }
    }
}
