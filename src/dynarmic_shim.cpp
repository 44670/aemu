#include <array>
#include <cstdint>
#include <cstring>
#include <memory>
#include <optional>

#include "dynarmic/interface/A32/a32.h"
#include "dynarmic/interface/A32/arch_version.h"
#include "dynarmic/interface/A32/config.h"
#include "dynarmic/interface/A32/coprocessor.h"
#include "dynarmic/interface/exclusive_monitor.h"
#include "dynarmic/interface/halt_reason.h"

extern "C" {

using AemuDynarmicRead8 = std::uint8_t (*)(void*, std::uint32_t, bool*);
using AemuDynarmicRead16 = std::uint16_t (*)(void*, std::uint32_t, bool*);
using AemuDynarmicRead32 = std::uint32_t (*)(void*, std::uint32_t, bool*);
using AemuDynarmicRead64 = std::uint64_t (*)(void*, std::uint32_t, bool*);
using AemuDynarmicWrite8 = bool (*)(void*, std::uint32_t, std::uint8_t);
using AemuDynarmicWrite16 = bool (*)(void*, std::uint32_t, std::uint16_t);
using AemuDynarmicWrite32 = bool (*)(void*, std::uint32_t, std::uint32_t);
using AemuDynarmicWrite64 = bool (*)(void*, std::uint32_t, std::uint64_t);

struct AemuDynarmicCallbacks {
    void* user;
    AemuDynarmicRead8 read8;
    AemuDynarmicRead16 read16;
    AemuDynarmicRead32 read32;
    AemuDynarmicRead64 read64;
    AemuDynarmicWrite8 write8;
    AemuDynarmicWrite16 write16;
    AemuDynarmicWrite32 write32;
    AemuDynarmicWrite64 write64;
};

struct AemuDynarmicStepResult {
    std::uint32_t halt_reason;
    std::uint32_t exception_pc;
    std::uint32_t memory_abort_addr;
    std::int32_t exception_kind;
    std::uint32_t svc;
    std::uint64_t ticks_used;
    bool svc_valid;
    bool memory_abort;
    bool interpreter_fallback;
};

}

struct AemuDynarmic;

class AemuDynarmicCp15 final : public Dynarmic::A32::Coprocessor {
public:
    explicit AemuDynarmicCp15(AemuDynarmic* owner)
            : owner(owner) {}

    std::optional<Callback> CompileInternalOperation(bool, unsigned, Dynarmic::A32::CoprocReg,
                                                     Dynarmic::A32::CoprocReg,
                                                     Dynarmic::A32::CoprocReg, unsigned) override {
        return std::nullopt;
    }

    CallbackOrAccessOneWord CompileSendOneWord(bool two, unsigned opc1, Dynarmic::A32::CoprocReg CRn,
                                               Dynarmic::A32::CoprocReg CRm, unsigned opc2) override;
    CallbackOrAccessTwoWords CompileSendTwoWords(bool, unsigned, Dynarmic::A32::CoprocReg) override {
        return std::monostate{};
    }
    CallbackOrAccessOneWord CompileGetOneWord(bool two, unsigned opc1, Dynarmic::A32::CoprocReg CRn,
                                              Dynarmic::A32::CoprocReg CRm, unsigned opc2) override;
    CallbackOrAccessTwoWords CompileGetTwoWords(bool two, unsigned opc, Dynarmic::A32::CoprocReg CRm) override;

    std::optional<Callback> CompileLoadWords(bool, bool, Dynarmic::A32::CoprocReg, std::optional<std::uint8_t>) override {
        return std::nullopt;
    }

    std::optional<Callback> CompileStoreWords(bool, bool, Dynarmic::A32::CoprocReg, std::optional<std::uint8_t>) override {
        return std::nullopt;
    }

private:
    static std::uint64_t Noop(void*, std::uint32_t, std::uint32_t);
    static std::uint64_t GetTpidrurw(void*, std::uint32_t, std::uint32_t);
    static std::uint64_t GetTpidruro(void*, std::uint32_t, std::uint32_t);
    static std::uint64_t SetTpidrurw(void*, std::uint32_t, std::uint32_t);
    static std::uint64_t GetVirtualCounter(void*, std::uint32_t, std::uint32_t);

    AemuDynarmic* owner;
};

class AemuDynarmicUserCallbacks final : public Dynarmic::A32::UserCallbacks {
public:
    explicit AemuDynarmicUserCallbacks(AemuDynarmicCallbacks callbacks)
            : callbacks(callbacks) {}

    void SetOwner(AemuDynarmic* value) {
        owner = value;
    }

    void SetUser(void* user) {
        callbacks.user = user;
    }

    void ResetStepState() {
        exception_kind = -1;
        exception_pc = 0;
        memory_abort_addr = 0;
        svc = 0;
        ticks_left = 1;
        ticks_used = 0;
        svc_valid = false;
        memory_abort = false;
        interpreter_fallback = false;
    }

    void ResetRunState(std::uint64_t ticks) {
        exception_kind = -1;
        exception_pc = 0;
        memory_abort_addr = 0;
        svc = 0;
        ticks_left = ticks;
        ticks_used = 0;
        svc_valid = false;
        memory_abort = false;
        interpreter_fallback = false;
    }

    std::optional<std::uint32_t> MemoryReadCode(std::uint32_t vaddr) override {
        bool ok = true;
        const std::uint32_t value = callbacks.read32(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
            return std::nullopt;
        }
        return value;
    }

    std::uint8_t MemoryRead8(std::uint32_t vaddr) override {
        bool ok = true;
        const auto value = callbacks.read8(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
        }
        return value;
    }

    std::uint16_t MemoryRead16(std::uint32_t vaddr) override {
        bool ok = true;
        const auto value = callbacks.read16(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
        }
        return value;
    }

    std::uint32_t MemoryRead32(std::uint32_t vaddr) override {
        bool ok = true;
        const auto value = callbacks.read32(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
        }
        return value;
    }

    std::uint64_t MemoryRead64(std::uint32_t vaddr) override {
        bool ok = true;
        const auto value = callbacks.read64(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
        }
        return value;
    }

    void MemoryWrite8(std::uint32_t vaddr, std::uint8_t value) override {
        if (!callbacks.write8(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
        }
    }

    void MemoryWrite16(std::uint32_t vaddr, std::uint16_t value) override {
        if (!callbacks.write16(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
        }
    }

    void MemoryWrite32(std::uint32_t vaddr, std::uint32_t value) override {
        if (!callbacks.write32(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
        }
    }

    void MemoryWrite64(std::uint32_t vaddr, std::uint64_t value) override {
        if (!callbacks.write64(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
        }
    }

    bool MemoryWriteExclusive8(std::uint32_t vaddr, std::uint8_t value, std::uint8_t expected) override {
        bool ok = true;
        const auto current = callbacks.read8(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
            return false;
        }
        if (current != expected) {
            return false;
        }
        if (!callbacks.write8(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
            return false;
        }
        return true;
    }

    bool MemoryWriteExclusive16(std::uint32_t vaddr, std::uint16_t value, std::uint16_t expected) override {
        bool ok = true;
        const auto current = callbacks.read16(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
            return false;
        }
        if (current != expected) {
            return false;
        }
        if (!callbacks.write16(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
            return false;
        }
        return true;
    }

    bool MemoryWriteExclusive32(std::uint32_t vaddr, std::uint32_t value, std::uint32_t expected) override {
        bool ok = true;
        const auto current = callbacks.read32(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
            return false;
        }
        if (current != expected) {
            return false;
        }
        if (!callbacks.write32(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
            return false;
        }
        return true;
    }

    bool MemoryWriteExclusive64(std::uint32_t vaddr, std::uint64_t value, std::uint64_t expected) override {
        bool ok = true;
        const auto current = callbacks.read64(callbacks.user, vaddr, &ok);
        if (!ok) {
            OnMemoryAbort(vaddr);
            return false;
        }
        if (current != expected) {
            return false;
        }
        if (!callbacks.write64(callbacks.user, vaddr, value)) {
            OnMemoryAbort(vaddr);
            return false;
        }
        return true;
    }

    void InterpreterFallback(std::uint32_t pc, size_t) override {
        interpreter_fallback = true;
        exception_pc = pc;
        Halt(Dynarmic::HaltReason::UserDefined2);
    }

    void CallSVC(std::uint32_t swi) override {
        svc = swi;
        svc_valid = true;
        Halt(Dynarmic::HaltReason::UserDefined1);
    }

    void ExceptionRaised(std::uint32_t pc, Dynarmic::A32::Exception exception) override {
        exception_pc = pc;
        exception_kind = static_cast<std::int32_t>(exception);
        Halt(Dynarmic::HaltReason::UserDefined1);
    }

    void AddTicks(std::uint64_t ticks) override {
        ticks_used += ticks;
        if (ticks > ticks_left) {
            ticks_left = 0;
        } else {
            ticks_left -= ticks;
        }
    }

    std::uint64_t GetTicksRemaining() override {
        return ticks_left;
    }

    AemuDynarmicCallbacks callbacks;
    AemuDynarmic* owner = nullptr;
    std::uint64_t ticks_left = 0;
    std::uint64_t ticks_used = 0;
    std::uint32_t exception_pc = 0;
    std::uint32_t memory_abort_addr = 0;
    std::int32_t exception_kind = -1;
    std::uint32_t svc = 0;
    bool svc_valid = false;
    bool memory_abort = false;
    bool interpreter_fallback = false;

private:
    void OnMemoryAbort(std::uint32_t vaddr);
    void Halt(Dynarmic::HaltReason reason);
};

struct AemuDynarmic {
    explicit AemuDynarmic(AemuDynarmicCallbacks callbacks, std::uint8_t** page_table)
            : callbacks(callbacks)
            , cp15(std::make_shared<AemuDynarmicCp15>(this)) {
        this->callbacks.SetOwner(this);
        Dynarmic::A32::UserConfig config{};
        config.callbacks = &this->callbacks;
        config.global_monitor = &monitor;
        config.arch_version = Dynarmic::A32::ArchVersion::v7;
        config.always_little_endian = true;
        config.enable_cycle_counting = true;
        config.wall_clock_cntpct = true;
        config.coprocessors[15] = cp15;
        if (page_table != nullptr) {
            config.page_table = reinterpret_cast<
                    std::array<std::uint8_t*, Dynarmic::A32::UserConfig::NUM_PAGE_TABLE_ENTRIES>*>(
                    page_table);
            config.absolute_offset_page_table = false;
            config.detect_misaligned_access_via_page_table = 8 | 16 | 32 | 64 | 128;
            config.only_detect_misalignment_via_page_table_on_page_boundary = true;
        }
        jit = std::make_unique<Dynarmic::A32::Jit>(config);
    }

    AemuDynarmicUserCallbacks callbacks;
    std::shared_ptr<AemuDynarmicCp15> cp15;
    Dynarmic::ExclusiveMonitor monitor{1};
    std::unique_ptr<Dynarmic::A32::Jit> jit;
    std::uint32_t cp15_tpidrurw = 0;
    std::uint32_t cp15_tpidruro = 0;
    std::uint64_t cp15_virtual_counter = 1;
};

AemuDynarmicCp15::CallbackOrAccessOneWord AemuDynarmicCp15::CompileSendOneWord(
        bool two, unsigned opc1, Dynarmic::A32::CoprocReg CRn, Dynarmic::A32::CoprocReg CRm, unsigned opc2) {
    if (two) {
        return std::monostate{};
    }
    if (opc1 == 0 && CRn == Dynarmic::A32::CoprocReg::C7
        && ((CRm == Dynarmic::A32::CoprocReg::C10 && (opc2 == 4 || opc2 == 5))
            || (CRm == Dynarmic::A32::CoprocReg::C5 && opc2 == 4))) {
        return Callback{&AemuDynarmicCp15::Noop, owner};
    }
    if (opc1 == 0 && CRn == Dynarmic::A32::CoprocReg::C13
        && CRm == Dynarmic::A32::CoprocReg::C0 && opc2 == 2) {
        return Callback{&AemuDynarmicCp15::SetTpidrurw, owner};
    }
    return std::monostate{};
}

AemuDynarmicCp15::CallbackOrAccessOneWord AemuDynarmicCp15::CompileGetOneWord(
        bool two, unsigned opc1, Dynarmic::A32::CoprocReg CRn, Dynarmic::A32::CoprocReg CRm, unsigned opc2) {
    if (two || opc1 != 0 || CRn != Dynarmic::A32::CoprocReg::C13 || CRm != Dynarmic::A32::CoprocReg::C0) {
        return std::monostate{};
    }
    if (opc2 == 2) {
        return Callback{&AemuDynarmicCp15::GetTpidrurw, owner};
    }
    if (opc2 == 3) {
        return Callback{&AemuDynarmicCp15::GetTpidruro, owner};
    }
    return std::monostate{};
}

AemuDynarmicCp15::CallbackOrAccessTwoWords AemuDynarmicCp15::CompileGetTwoWords(
        bool two, unsigned opc, Dynarmic::A32::CoprocReg CRm) {
    if (!two && opc == 1 && CRm == Dynarmic::A32::CoprocReg::C14) {
        return Callback{&AemuDynarmicCp15::GetVirtualCounter, owner};
    }
    return std::monostate{};
}

std::uint64_t AemuDynarmicCp15::Noop(void*, std::uint32_t, std::uint32_t) {
    return 0;
}

std::uint64_t AemuDynarmicCp15::GetTpidrurw(void* user, std::uint32_t, std::uint32_t) {
    return static_cast<AemuDynarmic*>(user)->cp15_tpidrurw;
}

std::uint64_t AemuDynarmicCp15::GetTpidruro(void* user, std::uint32_t, std::uint32_t) {
    return static_cast<AemuDynarmic*>(user)->cp15_tpidruro;
}

std::uint64_t AemuDynarmicCp15::SetTpidrurw(void* user, std::uint32_t value, std::uint32_t) {
    static_cast<AemuDynarmic*>(user)->cp15_tpidrurw = value;
    return 0;
}

std::uint64_t AemuDynarmicCp15::GetVirtualCounter(void* user, std::uint32_t, std::uint32_t) {
    auto* dynarmic = static_cast<AemuDynarmic*>(user);
    const std::uint64_t value = dynarmic->cp15_virtual_counter;
    dynarmic->cp15_virtual_counter += 1000;
    return value;
}

void AemuDynarmicUserCallbacks::Halt(Dynarmic::HaltReason reason) {
    if (owner && owner->jit) {
        owner->jit->HaltExecution(reason);
    }
}

void AemuDynarmicUserCallbacks::OnMemoryAbort(std::uint32_t vaddr) {
    memory_abort = true;
    memory_abort_addr = vaddr;
    Halt(Dynarmic::HaltReason::MemoryAbort);
}

extern "C" {

AemuDynarmic* aemu_dynarmic_new(AemuDynarmicCallbacks callbacks, std::uint8_t** page_table) {
    return new AemuDynarmic(callbacks, page_table);
}

void aemu_dynarmic_free(AemuDynarmic* dynarmic) {
    delete dynarmic;
}

void aemu_dynarmic_set_user(AemuDynarmic* dynarmic, void* user) {
    dynarmic->callbacks.SetUser(user);
}

void aemu_dynarmic_set_regs(AemuDynarmic* dynarmic, const std::uint32_t* regs16) {
    std::memcpy(dynarmic->jit->Regs().data(), regs16, sizeof(std::uint32_t) * 16);
}

void aemu_dynarmic_get_regs(const AemuDynarmic* dynarmic, std::uint32_t* regs16) {
    std::memcpy(regs16, dynarmic->jit->Regs().data(), sizeof(std::uint32_t) * 16);
}

void aemu_dynarmic_set_ext_regs(AemuDynarmic* dynarmic, const std::uint32_t* regs64) {
    std::memcpy(dynarmic->jit->ExtRegs().data(), regs64, sizeof(std::uint32_t) * 64);
}

void aemu_dynarmic_get_ext_regs(const AemuDynarmic* dynarmic, std::uint32_t* regs64) {
    std::memcpy(regs64, dynarmic->jit->ExtRegs().data(), sizeof(std::uint32_t) * 64);
}

void aemu_dynarmic_set_cpsr(AemuDynarmic* dynarmic, std::uint32_t value) {
    dynarmic->jit->SetCpsr(value);
}

std::uint32_t aemu_dynarmic_get_cpsr(const AemuDynarmic* dynarmic) {
    return dynarmic->jit->Cpsr();
}

void aemu_dynarmic_set_fpscr(AemuDynarmic* dynarmic, std::uint32_t value) {
    dynarmic->jit->SetFpscr(value);
}

std::uint32_t aemu_dynarmic_get_fpscr(const AemuDynarmic* dynarmic) {
    return dynarmic->jit->Fpscr();
}

void aemu_dynarmic_set_cp15(AemuDynarmic* dynarmic, std::uint32_t tpidrurw, std::uint32_t tpidruro, std::uint64_t virtual_counter) {
    dynarmic->cp15_tpidrurw = tpidrurw;
    dynarmic->cp15_tpidruro = tpidruro;
    dynarmic->cp15_virtual_counter = virtual_counter;
}

void aemu_dynarmic_get_cp15(const AemuDynarmic* dynarmic, std::uint32_t* tpidrurw, std::uint32_t* tpidruro, std::uint64_t* virtual_counter) {
    *tpidrurw = dynarmic->cp15_tpidrurw;
    *tpidruro = dynarmic->cp15_tpidruro;
    *virtual_counter = dynarmic->cp15_virtual_counter;
}

AemuDynarmicStepResult aemu_dynarmic_step(AemuDynarmic* dynarmic) {
    dynarmic->callbacks.ResetStepState();
    const Dynarmic::HaltReason halt_reason = dynarmic->jit->Step();
    AemuDynarmicStepResult result{};
    result.halt_reason = static_cast<std::uint32_t>(halt_reason);
    result.exception_pc = dynarmic->callbacks.exception_pc;
    result.memory_abort_addr = dynarmic->callbacks.memory_abort_addr;
    result.exception_kind = dynarmic->callbacks.exception_kind;
    result.svc = dynarmic->callbacks.svc_valid ? dynarmic->callbacks.svc : 0;
    result.ticks_used = dynarmic->callbacks.ticks_used;
    result.svc_valid = dynarmic->callbacks.svc_valid;
    result.memory_abort = dynarmic->callbacks.memory_abort;
    result.interpreter_fallback = dynarmic->callbacks.interpreter_fallback;
    return result;
}

AemuDynarmicStepResult aemu_dynarmic_run(AemuDynarmic* dynarmic, std::uint64_t ticks) {
    dynarmic->callbacks.ResetRunState(ticks);
    dynarmic->jit->ClearHalt(Dynarmic::HaltReason::Step | Dynarmic::HaltReason::MemoryAbort
                             | Dynarmic::HaltReason::CacheInvalidation | Dynarmic::HaltReason::UserDefined1
                             | Dynarmic::HaltReason::UserDefined2);
    const Dynarmic::HaltReason halt_reason = dynarmic->jit->Run();
    AemuDynarmicStepResult result{};
    result.halt_reason = static_cast<std::uint32_t>(halt_reason);
    result.exception_pc = dynarmic->callbacks.exception_pc;
    result.memory_abort_addr = dynarmic->callbacks.memory_abort_addr;
    result.exception_kind = dynarmic->callbacks.exception_kind;
    result.svc = dynarmic->callbacks.svc_valid ? dynarmic->callbacks.svc : 0;
    result.ticks_used = dynarmic->callbacks.ticks_used;
    result.svc_valid = dynarmic->callbacks.svc_valid;
    result.memory_abort = dynarmic->callbacks.memory_abort;
    result.interpreter_fallback = dynarmic->callbacks.interpreter_fallback;
    return result;
}

void aemu_dynarmic_clear_cache(AemuDynarmic* dynarmic) {
    dynarmic->jit->ClearCache();
}

void aemu_dynarmic_invalidate_cache_range(AemuDynarmic* dynarmic, std::uint32_t start, std::uintptr_t len) {
    dynarmic->jit->InvalidateCacheRange(start, static_cast<std::size_t>(len));
}

}
