	.file	"packed_scan_tile.c"
	.text
	.p2align	4                               # -- Begin function warm_b9_hot
	.type	warm_b9_hot,@function
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
                                        # -- End function
	.section	.rodata.cst32,"aM",@progbits,32
	.p2align	5, 0x0                          # -- Begin function warm_b9_scan_tile
.LCPI1_0:
	.quad	-1                              # 0xffffffffffffffff
	.quad	-1                              # 0xffffffffffffffff
	.quad	4294967295                      # 0xffffffff
	.quad	0                               # 0x0
	.text
	.globl	warm_b9_scan_tile
	.p2align	4
	.type	warm_b9_scan_tile,@function
warm_b9_scan_tile:                      # @warm_b9_scan_tile
# %bb.0:
	pushq	%rbp
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	subq	$200, %rsp
	testq	%rdx, %rdx
	sete	%al
	cmpq	$96, %rcx
	setne	%cl
	orb	%al, %cl
	movl	$2, %eax
	jne	.LBB1_46
# %bb.1:
	testb	$3, %dl
	setne	%cl
	cmpq	$-96, %rdx
	setae	%r8b
	orb	%cl, %r8b
	jne	.LBB1_46
# %bb.2:
	testq	%rdi, %rdi
	sete	%cl
	cmpq	$4696, %rsi                     # imm = 0x1258
	setne	%sil
	orb	%cl, %sil
	jne	.LBB1_4
# %bb.3:
	testb	$3, %dil
	sete	%cl
	cmpq	$-4696, %rdi                    # imm = 0xEDA8
	setb	%sil
	testb	%cl, %sil
	jne	.LBB1_5
.LBB1_4:
	movq	$-1, (%rdx)
	movl	$-1, 8(%rdx)
	vpcmpeqd	%ymm0, %ymm0, %ymm0
	vmovdqu	%ymm0, 12(%rdx)
	vmovaps	.LCPI1_0(%rip), %ymm0           # ymm0 = [18446744073709551615,18446744073709551615,4294967295,0]
	vmovups	%ymm0, 44(%rdx)
	vxorps	%xmm0, %xmm0, %xmm0
	vmovups	%xmm0, 76(%rdx)
	movl	$0, 92(%rdx)
.LBB1_46:
	addq	$200, %rsp
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15
	popq	%rbp
	vzeroupper
	retq
.LBB1_5:
	leaq	96(%rdx), %rcx
	cmpq	%rdi, %rcx
	seta	%cl
	leaq	4696(%rdi), %rsi
	cmpq	%rdx, %rsi
	seta	%sil
	testb	%sil, %cl
	jne	.LBB1_46
# %bb.6:
	movq	$-1, (%rdx)
	movl	$-1, 8(%rdx)
	vpcmpeqd	%ymm0, %ymm0, %ymm0
	vmovdqu	%ymm0, 12(%rdx)
	vmovaps	.LCPI1_0(%rip), %ymm0           # ymm0 = [18446744073709551615,18446744073709551615,4294967295,0]
	vmovups	%ymm0, 44(%rdx)
	vxorps	%xmm0, %xmm0, %xmm0
	vmovups	%xmm0, 76(%rdx)
	movl	$0, 92(%rdx)
	cmpl	$1111031809, (%rdi)             # imm = 0x42390001
	jne	.LBB1_46
# %bb.7:
	cmpl	$100, 4(%rdi)
	jne	.LBB1_46
# %bb.8:
	movl	8(%rdi), %esi
	andnl	12(%rdi), %esi, %ecx
	jne	.LBB1_46
# %bb.9:
	movl	16(%rdi), %ecx
	cmpl	$255, %ecx
	ja	.LBB1_46
# %bb.10:
	andnl	20(%rdi), %ecx, %r8d
	jne	.LBB1_46
# %bb.11:
	movq	%rdx, %r13
	movq	%rdi, %rbx
	movq	%rdx, %rdi
	movl	%ecx, %edx
	vzeroupper
	callq	b9_all_candidates
	movq	%rbx, %rcx
	xorl	%r14d, %r14d
	movl	$1, %r15d
	xorl	%r12d, %r12d
.LBB1_13:                               # =>This Loop Header: Depth=1
                                        #     Child Loop BB1_14 Depth 2
	movq	%r12, %rax
	andq	$-16, %rax
	movl	%r12d, %edx
	andl	$15, %edx
	shlq	$2, %rdx
	leaq	(%rdx,%rax,4), %rdx
	addq	%rcx, %rdx
	movl	$27, %esi
	movl	$1, %ebp
	movl	$2, %eax
.LBB1_14:                               #   Parent Loop BB1_13 Depth=1
                                        # =>  This Inner Loop Header: Depth=2
	movzbl	-3(%rdx,%rsi), %edi
	testb	%dil, %dil
	je	.LBB1_46
# %bb.15:                               #   in Loop: Header=BB1_14 Depth=2
	movzbl	-2(%rdx,%rsi), %r8d
	testb	%r8b, %r8b
	je	.LBB1_46
# %bb.16:                               #   in Loop: Header=BB1_14 Depth=2
	movzbl	-1(%rdx,%rsi), %r10d
	testb	%r10b, %r10b
	je	.LBB1_46
# %bb.17:                               #   in Loop: Header=BB1_14 Depth=2
	movzbl	(%rdx,%rsi), %r9d
	testb	%r9b, %r9b
	je	.LBB1_46
# %bb.18:                               #   in Loop: Header=BB1_14 Depth=2
	addb	$-128, %r10b
	addb	$-128, %r8b
	addb	$-128, %dil
	orb	%r8b, %dil
	orb	%r10b, %dil
	addb	$-128, %r9b
	orb	%dil, %r9b
	cmovnel	%r14d, %ebp
	subq	$-128, %rsi
	cmpq	$3227, %rsi                     # imm = 0xC9B
	jne	.LBB1_14
# %bb.19:                               #   in Loop: Header=BB1_13 Depth=1
	movl	4024(%rcx,%r12,4), %edi
	movl	4152(%rcx,%r12,4), %esi
	movl	4280(%rcx,%r12,4), %edx
	movl	4408(%rcx,%r12,4), %ecx
	movl	%ebp, %r8d
	callq	b9_metadata_status
	testl	%eax, %eax
	jne	.LBB1_46
# %bb.20:                               #   in Loop: Header=BB1_13 Depth=1
	shlxl	%r12d, %r15d, %eax
	andl	12(%rbx), %eax
	orl	%ebp, %eax
	je	.LBB1_21
# %bb.12:                               #   in Loop: Header=BB1_13 Depth=1
	movq	%rbx, %rcx
	incq	%r12
	cmpq	$32, %r12
	jne	.LBB1_13
# %bb.22:
	leaq	3228(%rcx), %r15
	movq	$0, 8(%rsp)                     # 8-byte Folded Spill
	movl	$2, %r14d
.LBB1_23:                               # =>This Loop Header: Depth=1
                                        #     Child Loop BB1_24 Depth 2
	movl	$1, %ebp
	xorl	%r12d, %r12d
	xorl	%eax, %eax
.LBB1_24:                               #   Parent Loop BB1_23 Depth=1
                                        # =>  This Inner Loop Header: Depth=2
	movsbl	-4(%r15,%rax), %ecx
	cmpl	$-128, %ecx
	je	.LBB1_45
# %bb.25:                               #   in Loop: Header=BB1_24 Depth=2
	movsbl	-3(%r15,%rax), %edi
	cmpl	$-128, %edi
	je	.LBB1_45
# %bb.26:                               #   in Loop: Header=BB1_24 Depth=2
	movsbl	-2(%r15,%rax), %esi
	cmpl	$-128, %esi
	je	.LBB1_45
# %bb.27:                               #   in Loop: Header=BB1_24 Depth=2
	movsbl	-1(%r15,%rax), %edx
	cmpl	$-128, %edx
	je	.LBB1_45
# %bb.28:                               #   in Loop: Header=BB1_24 Depth=2
	movsbl	(%r15,%rax), %r8d
	cmpl	$-128, %r8d
	je	.LBB1_45
# %bb.29:                               #   in Loop: Header=BB1_24 Depth=2
	addl	%ecx, %r12d
	movl	%r12d, %r10d
	addl	%edi, %r10d
	addl	%esi, %r10d
	orl	%ecx, %edi
	orl	%edx, %esi
	orl	%edi, %esi
	orb	%r8b, %sil
	movl	$0, %ecx
	cmovnel	%ecx, %ebp
	addl	%r8d, %edx
	movl	%edx, %r12d
	addl	%r10d, %r12d
	addq	$5, %rax
	cmpq	$100, %rax
	jne	.LBB1_24
# %bb.30:                               #   in Loop: Header=BB1_23 Depth=1
	movq	8(%rsp), %rcx                   # 8-byte Reload
	movl	4536(%rbx,%rcx,4), %edi
	movl	4568(%rbx,%rcx,4), %esi
	movl	4600(%rbx,%rcx,4), %edx
	movl	4632(%rbx,%rcx,4), %ecx
	movl	%edi, 24(%rsp)                  # 4-byte Spill
	movl	%esi, 20(%rsp)                  # 4-byte Spill
	movl	%ecx, 28(%rsp)                  # 4-byte Spill
	movl	%ebp, %r8d
	callq	b9_metadata_status
	testl	%eax, %eax
	jne	.LBB1_31
# %bb.32:                               #   in Loop: Header=BB1_23 Depth=1
	movq	8(%rsp), %rax                   # 8-byte Reload
	movl	4664(%rbx,%rax,4), %eax
	movl	%eax, %ecx
	notl	%ecx
	testl	$2139095040, %ecx               # imm = 0x7F800000
	je	.LBB1_45
# %bb.33:                               #   in Loop: Header=BB1_23 Depth=1
	testl	%eax, %eax
	setne	%cl
	leal	-1417674753(%rax), %edx
	cmpl	$-1006632961, %edx              # imm = 0xC3FFFFFF
	setb	%dl
	testb	%dl, %cl
	jne	.LBB1_34
# %bb.35:                               #   in Loop: Header=BB1_23 Depth=1
	testl	%eax, %eax
	setne	%cl
	movl	20(%rbx), %esi
	testl	%ebp, %ebp
	sete	%dl
	movq	8(%rsp), %rdi                   # 8-byte Reload
	btl	%edi, %esi
	jb	.LBB1_37
# %bb.36:                               #   in Loop: Header=BB1_23 Depth=1
	orb	%cl, %dl
	jne	.LBB1_45
.LBB1_37:                               #   in Loop: Header=BB1_23 Depth=1
	movq	8(%rsp), %rdx                   # 8-byte Reload
	movq	%rdx, %rcx
	shlq	$4, %rcx
	movl	24(%rsp), %esi                  # 4-byte Reload
	movl	%esi, 64(%rsp,%rcx)
	movl	20(%rsp), %esi                  # 4-byte Reload
	movl	%esi, 68(%rsp,%rcx)
	shll	$7, %r12d
	movl	%r12d, 76(%rsp,%rcx)
	vmovd	%eax, %xmm0
	vpxor	%xmm1, %xmm1, %xmm1
	vpblendd	$1, %xmm0, %xmm1, %xmm0         # xmm0 = xmm0[0],xmm1[1,2,3]
	vsqrtss	{ru-sae}, %xmm0, %xmm0, %xmm0
	vmovss	28(%rsp), %xmm1                 # 4-byte Reload
                                        # xmm1 = mem[0],zero,zero,zero
	vaddss	{ru-sae}, %xmm1, %xmm0, %xmm0
	vmovss	%xmm0, 72(%rsp,%rcx)
	incq	%rdx
	addq	$100, %r15
	movq	%rdx, 8(%rsp)                   # 8-byte Spill
	cmpq	$8, %rdx
	jne	.LBB1_23
# %bb.38:
	leaq	64(%rsp), %rsi
	leaq	32(%rsp), %rdx
	movq	%rbx, %rdi
	callq	warm_b9_hot
	xorl	%eax, %eax
	movl	$1, %ecx
	xorl	%r14d, %r14d
	jmp	.LBB1_39
.LBB1_21:
	movl	$2, %eax
	jmp	.LBB1_46
.LBB1_43:                               #   in Loop: Header=BB1_39 Depth=1
	movl	12(%rbx), %esi
.LBB1_44:                               #   in Loop: Header=BB1_39 Depth=1
	movl	32(%rsp,%rax,4), %edi
	andl	%edx, %edi
	andl	%esi, %edi
	movl	%edi, 64(%r13,%rax,4)
	xorl	%edx, %edi
	movl	%edi, (%r13,%rax,4)
	andnl	%edx, %esi, %edx
	movl	%edx, 32(%r13,%rax,4)
	cmovnel	%ecx, %r14d
	incq	%rax
	cmpq	$8, %rax
	je	.LBB1_45
.LBB1_39:                               # =>This Inner Loop Header: Depth=1
	movl	16(%rbx), %edx
	btl	%eax, %edx
	movl	$0, %edx
	jae	.LBB1_41
# %bb.40:                               #   in Loop: Header=BB1_39 Depth=1
	movl	8(%rbx), %edx
.LBB1_41:                               #   in Loop: Header=BB1_39 Depth=1
	shlxl	%eax, %ecx, %esi
	testl	%esi, 20(%rbx)
	jne	.LBB1_43
# %bb.42:                               #   in Loop: Header=BB1_39 Depth=1
	xorl	%esi, %esi
	jmp	.LBB1_44
.LBB1_31:
	movl	%eax, %r14d
	jmp	.LBB1_45
.LBB1_34:
	movl	$1, %r14d
.LBB1_45:
	movl	%r14d, %eax
	jmp	.LBB1_46
.Lfunc_end1:
	.size	warm_b9_scan_tile, .Lfunc_end1-warm_b9_scan_tile
                                        # -- End function
	.p2align	4                               # -- Begin function b9_all_candidates
	.type	b9_all_candidates,@function
b9_all_candidates:                      # @b9_all_candidates
# %bb.0:
	pushq	%rbx
	movl	%edx, %eax
	movl	%edx, %ecx
	movl	%edx, %r8d
	movl	%edx, %r9d
	movl	%edx, %r10d
	movl	%edx, %r11d
	movsbl	%dl, %ebx
	andl	$1, %edx
	negl	%edx
	andl	%esi, %edx
	movl	%edx, (%rdi)
	movl	%edx, 32(%rdi)
	movl	$0, 64(%rdi)
	shll	$30, %eax
	sarl	$31, %eax
	andl	%esi, %eax
	movl	%eax, 4(%rdi)
	movl	%eax, 36(%rdi)
	movq	$0, 68(%rdi)
	shll	$29, %ecx
	sarl	$31, %ecx
	andl	%esi, %ecx
	movl	%ecx, 8(%rdi)
	movl	%ecx, 40(%rdi)
	shll	$28, %r8d
	sarl	$31, %r8d
	andl	%esi, %r8d
	movl	%r8d, 12(%rdi)
	movl	%r8d, 44(%rdi)
	vxorps	%xmm0, %xmm0, %xmm0
	vmovups	%xmm0, 76(%rdi)
	shll	$27, %r9d
	sarl	$31, %r9d
	andl	%esi, %r9d
	movl	%r9d, 16(%rdi)
	movl	%r9d, 48(%rdi)
	shll	$26, %r10d
	sarl	$31, %r10d
	andl	%esi, %r10d
	movl	%r10d, 20(%rdi)
	movl	%r10d, 52(%rdi)
	shll	$25, %r11d
	sarl	$31, %r11d
	andl	%esi, %r11d
	movl	%r11d, 24(%rdi)
	movl	%r11d, 56(%rdi)
	sarl	$7, %ebx
	andl	%esi, %ebx
	movl	%ebx, 28(%rdi)
	movl	%ebx, 60(%rdi)
	movl	$0, 92(%rdi)
	popq	%rbx
	retq
.Lfunc_end2:
	.size	b9_all_candidates, .Lfunc_end2-b9_all_candidates
                                        # -- End function
	.p2align	4                               # -- Begin function b9_metadata_status
	.type	b9_metadata_status,@function
b9_metadata_status:                     # @b9_metadata_status
# %bb.0:
                                        # kill: def $edx killed $edx def $rdx
                                        # kill: def $esi killed $esi def $rsi
	movl	%edi, %eax
	notl	%eax
	testl	$2139095040, %eax               # imm = 0x7F800000
	sete	%al
	movl	%esi, %r9d
	notl	%r9d
	testl	$2139095040, %r9d               # imm = 0x7F800000
	sete	%r9b
	orb	%al, %r9b
	movl	%edx, %eax
	notl	%eax
	testl	$2139095040, %eax               # imm = 0x7F800000
	sete	%al
	movl	%ecx, %r10d
	notl	%r10d
	testl	$2139095040, %r10d              # imm = 0x7F800000
	sete	%r10b
	orb	%al, %r10b
	orb	%r9b, %r10b
	movl	$2, %eax
	je	.LBB3_1
.LBB3_5:
	retq
.LBB3_1:
	testl	%r8d, %r8d
	je	.LBB3_3
# %bb.2:
	xorl	$8388608, %edi                  # imm = 0x800000
	orl	%esi, %edx
	orl	%ecx, %edx
	xorl	%eax, %eax
	orl	%edi, %edx
	setne	%al
	retq
.LBB3_3:
	addl	$-1149239297, %edi              # imm = 0xBB7FFFFF
	cmpl	$-285212673, %edi               # imm = 0xEEFFFFFF
	setb	%al
	leal	-1409286145(%rsi), %edi
	cmpl	$-746586113, %edi               # imm = 0xD37FFFFF
	setb	%dil
	orb	%al, %dil
	leal	-1409286145(%rdx), %eax
	cmpl	$-746586113, %eax               # imm = 0xD37FFFFF
	setb	%al
	orb	%dil, %al
	movl	$1, %eax
	jne	.LBB3_5
# %bb.4:
	testl	%ecx, %ecx
	setne	%al
	addl	$-1233125377, %ecx              # imm = 0xB67FFFFF
	cmpl	$-553648129, %ecx               # imm = 0xDEFFFFFF
	setb	%cl
	andb	%al, %cl
	cmpl	%edx, %esi
	seta	%al
	orb	%cl, %al
	movzbl	%al, %eax
	retq
.Lfunc_end3:
	.size	b9_metadata_status, .Lfunc_end3-b9_metadata_status
                                        # -- End function
	.ident	"Apple clang version 21.0.0 (clang-2100.4.1.1)"
	.section	".note.GNU-stack","",@progbits
	.addrsig
	.addrsig_sym warm_b9_hot
