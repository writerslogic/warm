warm_b9_hot:                            # @warm_b9_hot
# %bb.0:
	leaq	88(%rdi), %rax
	vpxord	%xmm21, %xmm21, %xmm21
	movq	$-100, %rcx
	vpxord	%xmm20, %xmm20, %xmm20
	vpxord	%xmm19, %xmm19, %xmm19
	vpxord	%xmm18, %xmm18, %xmm18
	vpxord	%xmm17, %xmm17, %xmm17
	vpxord	%xmm16, %xmm16, %xmm16
	vpxor	%xmm15, %xmm15, %xmm15
	vpxor	%xmm14, %xmm14, %xmm14
	vpxor	%xmm10, %xmm10, %xmm10
	vpxor	%xmm6, %xmm6, %xmm6
	vpxor	%xmm5, %xmm5, %xmm5
	vpxor	%xmm4, %xmm4, %xmm4
	vpxor	%xmm3, %xmm3, %xmm3
	vpxor	%xmm2, %xmm2, %xmm2
	vpxor	%xmm1, %xmm1, %xmm1
	vpxor	%xmm0, %xmm0, %xmm0
	.p2align	4
.LBB0_1:                                # =>This Inner Loop Header: Depth=1
	vmovdqu64	-64(%rax), %zmm7
	vmovdqu64	(%rax), %zmm8
	vpbroadcastd	3324(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm21
	vpdpbusd	%zmm9, %zmm8, %zmm20
	vpbroadcastd	3424(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm19
	vpdpbusd	%zmm9, %zmm8, %zmm18
	vpbroadcastd	3524(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm17
	vpdpbusd	%zmm9, %zmm8, %zmm16
	vpbroadcastd	3624(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm15
	vpdpbusd	%zmm9, %zmm8, %zmm14
	vpbroadcastd	3724(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm10
	vpdpbusd	%zmm9, %zmm8, %zmm6
	vpbroadcastd	3824(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm5
	vpdpbusd	%zmm9, %zmm8, %zmm4
	vpbroadcastd	3924(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm3
	vpdpbusd	%zmm9, %zmm8, %zmm2
	vpbroadcastd	4024(%rdi,%rcx), %zmm9
	vpdpbusd	%zmm9, %zmm7, %zmm1
	vpdpbusd	%zmm9, %zmm8, %zmm0
	subq	$-128, %rax
	addq	$4, %rcx
	jne	.LBB0_1
# %bb.2:
	vmovups	4024(%rdi), %zmm12
	vmovups	4088(%rdi), %zmm7
	vmovups	4152(%rdi), %zmm13
	vmovups	4216(%rdi), %zmm8
	vmovups	4408(%rdi), %zmm11
	vmovups	4472(%rdi), %zmm9
	movl	12(%rsi), %eax
	vpbroadcastd	%eax, %zmm22
	vpsubd	%zmm22, %zmm21, %zmm21
	vpaddd	%zmm21, %zmm21, %zmm21
	vcvtdq2ps	%zmm21, %zmm21
	vbroadcastss	(%rsi), %zmm23
	vmulps	{ru-sae}, %zmm21, %zmm23, %zmm21
	vmulps	{ru-sae}, %zmm21, %zmm12, %zmm21
	vbroadcastss	4(%rsi), %zmm24
	vaddps	{rd-sae}, %zmm24, %zmm13, %zmm25
	vsubps	{rd-sae}, %zmm21, %zmm25, %zmm21
	vbroadcastss	8(%rsi), %zmm25
	vaddps	{ru-sae}, %zmm11, %zmm25, %zmm26
	vmulps	{ru-sae}, %zmm26, %zmm26, %zmm26
	vcmpgt_oqps	%zmm26, %zmm21, %k0
	vpsubd	%zmm22, %zmm20, %zmm20
	vpaddd	%zmm20, %zmm20, %zmm20
	vcvtdq2ps	%zmm20, %zmm20
	vmulps	{ru-sae}, %zmm20, %zmm23, %zmm20
	vmulps	{ru-sae}, %zmm20, %zmm7, %zmm20
	vaddps	{rd-sae}, %zmm24, %zmm8, %zmm21
	vsubps	{rd-sae}, %zmm20, %zmm21, %zmm20
	vaddps	{ru-sae}, %zmm9, %zmm25, %zmm21
	vmulps	{ru-sae}, %zmm21, %zmm21, %zmm21
	vcmpgt_oqps	%zmm21, %zmm20, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, (%rdx)
	movl	28(%rsi), %eax
	vpbroadcastd	%eax, %zmm20
	vpsubd	%zmm20, %zmm19, %zmm19
	vpaddd	%zmm19, %zmm19, %zmm19
	vcvtdq2ps	%zmm19, %zmm19
	vbroadcastss	16(%rsi), %zmm21
	vmulps	{ru-sae}, %zmm19, %zmm21, %zmm19
	vmulps	{ru-sae}, %zmm19, %zmm12, %zmm19
	vbroadcastss	20(%rsi), %zmm22
	vaddps	{rd-sae}, %zmm22, %zmm13, %zmm23
	vbroadcastss	24(%rsi), %zmm24
	vsubps	{rd-sae}, %zmm19, %zmm23, %zmm19
	vaddps	{ru-sae}, %zmm11, %zmm24, %zmm23
	vmulps	{ru-sae}, %zmm23, %zmm23, %zmm23
	vcmpgt_oqps	%zmm23, %zmm19, %k0
	vpsubd	%zmm20, %zmm18, %zmm18
	vpaddd	%zmm18, %zmm18, %zmm18
	vcvtdq2ps	%zmm18, %zmm18
	vmulps	{ru-sae}, %zmm18, %zmm21, %zmm18
	vmulps	{ru-sae}, %zmm18, %zmm7, %zmm18
	vaddps	{rd-sae}, %zmm22, %zmm8, %zmm19
	vsubps	{rd-sae}, %zmm18, %zmm19, %zmm18
	vaddps	{ru-sae}, %zmm9, %zmm24, %zmm19
	vmulps	{ru-sae}, %zmm19, %zmm19, %zmm19
	vcmpgt_oqps	%zmm19, %zmm18, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 4(%rdx)
	movl	44(%rsi), %eax
	vpbroadcastd	%eax, %zmm18
	vpsubd	%zmm18, %zmm17, %zmm17
	vpaddd	%zmm17, %zmm17, %zmm17
	vcvtdq2ps	%zmm17, %zmm17
	vbroadcastss	32(%rsi), %zmm19
	vmulps	{ru-sae}, %zmm17, %zmm19, %zmm17
	vmulps	{ru-sae}, %zmm17, %zmm12, %zmm17
	vbroadcastss	36(%rsi), %zmm20
	vaddps	{rd-sae}, %zmm20, %zmm13, %zmm21
	vsubps	{rd-sae}, %zmm17, %zmm21, %zmm17
	vbroadcastss	40(%rsi), %zmm21
	vaddps	{ru-sae}, %zmm11, %zmm21, %zmm22
	vmulps	{ru-sae}, %zmm22, %zmm22, %zmm22
	vcmpgt_oqps	%zmm22, %zmm17, %k0
	vpsubd	%zmm18, %zmm16, %zmm16
	vpaddd	%zmm16, %zmm16, %zmm16
	vcvtdq2ps	%zmm16, %zmm16
	vmulps	{ru-sae}, %zmm16, %zmm19, %zmm16
	vmulps	{ru-sae}, %zmm16, %zmm7, %zmm16
	vaddps	{rd-sae}, %zmm20, %zmm8, %zmm17
	vsubps	{rd-sae}, %zmm16, %zmm17, %zmm16
	vaddps	{ru-sae}, %zmm9, %zmm21, %zmm17
	vmulps	{ru-sae}, %zmm17, %zmm17, %zmm17
	vcmpgt_oqps	%zmm17, %zmm16, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 8(%rdx)
	movl	60(%rsi), %eax
	vpbroadcastd	%eax, %zmm16
	vpsubd	%zmm16, %zmm15, %zmm15
	vpaddd	%zmm15, %zmm15, %zmm15
	vcvtdq2ps	%zmm15, %zmm15
	vbroadcastss	48(%rsi), %zmm17
	vmulps	{ru-sae}, %zmm15, %zmm17, %zmm15
	vmulps	{ru-sae}, %zmm15, %zmm12, %zmm15
	vbroadcastss	52(%rsi), %zmm18
	vaddps	{rd-sae}, %zmm18, %zmm13, %zmm19
	vsubps	{rd-sae}, %zmm15, %zmm19, %zmm15
	vbroadcastss	56(%rsi), %zmm19
	vaddps	{ru-sae}, %zmm11, %zmm19, %zmm20
	vmulps	{ru-sae}, %zmm20, %zmm20, %zmm20
	vcmpgt_oqps	%zmm20, %zmm15, %k0
	vpsubd	%zmm16, %zmm14, %zmm14
	vpaddd	%zmm14, %zmm14, %zmm14
	vcvtdq2ps	%zmm14, %zmm14
	vmulps	{ru-sae}, %zmm14, %zmm17, %zmm14
	vmulps	{ru-sae}, %zmm14, %zmm7, %zmm14
	vaddps	{rd-sae}, %zmm18, %zmm8, %zmm15
	vsubps	{rd-sae}, %zmm14, %zmm15, %zmm14
	vaddps	{ru-sae}, %zmm9, %zmm19, %zmm15
	vmulps	{ru-sae}, %zmm15, %zmm15, %zmm15
	vcmpgt_oqps	%zmm15, %zmm14, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 12(%rdx)
	movl	76(%rsi), %eax
	vpbroadcastd	%eax, %zmm14
	vpsubd	%zmm14, %zmm10, %zmm10
	vpaddd	%zmm10, %zmm10, %zmm10
	vcvtdq2ps	%zmm10, %zmm10
	vbroadcastss	64(%rsi), %zmm15
	vmulps	{ru-sae}, %zmm10, %zmm15, %zmm10
	vbroadcastss	68(%rsi), %zmm16
	vmulps	{ru-sae}, %zmm10, %zmm12, %zmm10
	vaddps	{rd-sae}, %zmm16, %zmm13, %zmm17
	vsubps	{rd-sae}, %zmm10, %zmm17, %zmm10
	vbroadcastss	72(%rsi), %zmm17
	vaddps	{ru-sae}, %zmm11, %zmm17, %zmm18
	vmulps	{ru-sae}, %zmm18, %zmm18, %zmm18
	vcmpgt_oqps	%zmm18, %zmm10, %k0
	vpsubd	%zmm14, %zmm6, %zmm6
	vpaddd	%zmm6, %zmm6, %zmm6
	vcvtdq2ps	%zmm6, %zmm6
	vmulps	{ru-sae}, %zmm6, %zmm15, %zmm6
	vmulps	{ru-sae}, %zmm6, %zmm7, %zmm6
	vaddps	{rd-sae}, %zmm16, %zmm8, %zmm10
	vsubps	{rd-sae}, %zmm6, %zmm10, %zmm6
	vaddps	{ru-sae}, %zmm9, %zmm17, %zmm10
	vmulps	{ru-sae}, %zmm10, %zmm10, %zmm10
	vcmpgt_oqps	%zmm10, %zmm6, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 16(%rdx)
	movl	92(%rsi), %eax
	vpbroadcastd	%eax, %zmm6
	vpsubd	%zmm6, %zmm5, %zmm5
	vpaddd	%zmm5, %zmm5, %zmm5
	vcvtdq2ps	%zmm5, %zmm5
	vbroadcastss	80(%rsi), %zmm10
	vmulps	{ru-sae}, %zmm5, %zmm10, %zmm5
	vmulps	{ru-sae}, %zmm5, %zmm12, %zmm5
	vbroadcastss	84(%rsi), %zmm14
	vaddps	{rd-sae}, %zmm14, %zmm13, %zmm15
	vsubps	{rd-sae}, %zmm5, %zmm15, %zmm5
	vbroadcastss	88(%rsi), %zmm15
	vaddps	{ru-sae}, %zmm11, %zmm15, %zmm16
	vmulps	{ru-sae}, %zmm16, %zmm16, %zmm16
	vcmpgt_oqps	%zmm16, %zmm5, %k0
	vpsubd	%zmm6, %zmm4, %zmm4
	vpaddd	%zmm4, %zmm4, %zmm4
	vcvtdq2ps	%zmm4, %zmm4
	vmulps	{ru-sae}, %zmm4, %zmm10, %zmm4
	vmulps	{ru-sae}, %zmm4, %zmm7, %zmm4
	vaddps	{rd-sae}, %zmm14, %zmm8, %zmm5
	vsubps	{rd-sae}, %zmm4, %zmm5, %zmm4
	vaddps	{ru-sae}, %zmm9, %zmm15, %zmm5
	vmulps	{ru-sae}, %zmm5, %zmm5, %zmm5
	vcmpgt_oqps	%zmm5, %zmm4, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 20(%rdx)
	movl	108(%rsi), %eax
	vpbroadcastd	%eax, %zmm4
	vpsubd	%zmm4, %zmm3, %zmm3
	vpaddd	%zmm3, %zmm3, %zmm3
	vcvtdq2ps	%zmm3, %zmm3
	vbroadcastss	96(%rsi), %zmm5
	vmulps	{ru-sae}, %zmm3, %zmm5, %zmm3
	vmulps	{ru-sae}, %zmm3, %zmm12, %zmm3
	vbroadcastss	100(%rsi), %zmm6
	vaddps	{rd-sae}, %zmm6, %zmm13, %zmm10
	vbroadcastss	104(%rsi), %zmm14
	vsubps	{rd-sae}, %zmm3, %zmm10, %zmm3
	vaddps	{ru-sae}, %zmm11, %zmm14, %zmm10
	vmulps	{ru-sae}, %zmm10, %zmm10, %zmm10
	vcmpgt_oqps	%zmm10, %zmm3, %k0
	vpsubd	%zmm4, %zmm2, %zmm2
	vpaddd	%zmm2, %zmm2, %zmm2
	vcvtdq2ps	%zmm2, %zmm2
	vmulps	{ru-sae}, %zmm2, %zmm5, %zmm2
	vmulps	{ru-sae}, %zmm2, %zmm7, %zmm2
	vaddps	{rd-sae}, %zmm6, %zmm8, %zmm3
	vsubps	{rd-sae}, %zmm2, %zmm3, %zmm2
	vaddps	{ru-sae}, %zmm9, %zmm14, %zmm3
	vmulps	{ru-sae}, %zmm3, %zmm3, %zmm3
	vcmpgt_oqps	%zmm3, %zmm2, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 24(%rdx)
	movl	124(%rsi), %eax
	vpbroadcastd	%eax, %zmm2
	vpsubd	%zmm2, %zmm1, %zmm1
	vpaddd	%zmm1, %zmm1, %zmm1
	vcvtdq2ps	%zmm1, %zmm1
	vbroadcastss	112(%rsi), %zmm3
	vmulps	{ru-sae}, %zmm1, %zmm3, %zmm1
	vmulps	{ru-sae}, %zmm1, %zmm12, %zmm1
	vbroadcastss	116(%rsi), %zmm4
	vaddps	{rd-sae}, %zmm4, %zmm13, %zmm5
	vsubps	{rd-sae}, %zmm1, %zmm5, %zmm1
	vbroadcastss	120(%rsi), %zmm5
	vaddps	{ru-sae}, %zmm11, %zmm5, %zmm6
	vmulps	{ru-sae}, %zmm6, %zmm6, %zmm6
	vcmpgt_oqps	%zmm6, %zmm1, %k0
	vpsubd	%zmm2, %zmm0, %zmm0
	vpaddd	%zmm0, %zmm0, %zmm0
	vcvtdq2ps	%zmm0, %zmm0
	vmulps	{ru-sae}, %zmm0, %zmm3, %zmm0
	vmulps	{ru-sae}, %zmm0, %zmm7, %zmm0
	vaddps	{rd-sae}, %zmm4, %zmm8, %zmm1
	vsubps	{rd-sae}, %zmm0, %zmm1, %zmm0
	vaddps	{ru-sae}, %zmm9, %zmm5, %zmm1
	vmulps	{ru-sae}, %zmm1, %zmm1, %zmm1
	vcmpgt_oqps	%zmm1, %zmm0, %k1
	kunpckwd	%k0, %k1, %k0
	kmovd	%k0, 28(%rdx)
	vzeroupper
	retq
.Lfunc_end0:
	.size	warm_b9_hot, .Lfunc_end0-warm_b9_hot
