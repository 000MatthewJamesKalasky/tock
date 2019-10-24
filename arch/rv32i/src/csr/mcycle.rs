use kernel::common::registers::register_bitfields;

// mtvec contains the address(es) of the trap handler
register_bitfields![u32,
mcycle [
    mcycle OFFSET(0) NUMBITS(32) []
]
];
