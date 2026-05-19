#!/usr/bin/env python3
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time


DEFAULT_APK = pathlib.Path("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk")
DEFAULT_BINARY = pathlib.Path("target/release/aemu")
DEFAULT_ABI = "armeabi-v7a"
DEFAULT_OUT_DIR = pathlib.Path("tmp/mcpe-smoke")
DEFAULT_STEPS = 600_000_000
MCPE_LIBRARY = "libminecraftpe.so"
ARMV7_NEON_HWCAP = 0x0008B0D7
ARMV7_NO_NEON_HWCAP = 0x0008A0D7

MCPE_NATIVE_TRACE_PRESETS = {
    "render-loop": {
        "description": "trace MCPE GameRenderer render/resource-ready path for no-draw diagnosis",
        "events": [
            (0x009CC754, "GameRenderer::render.entry"),
            (0x009CC8F2, "GameRenderer::render.resource-ready-gate"),
            (0x009CD48C, "GameRenderer::_checkAndDrawInputUI.entry"),
            (0x006BEB1C, "MinecraftClient::onResourcesLoaded.entry"),
        ],
        "mem32": [
            (0x009CC8F2, "r7+0x238,+0x23c,+0x240,+0x244"),
        ],
        "bytes": [
            (0x009CC8F2, "r7+0x238,16"),
        ],
        "event_limit": 200,
    },
    "resource-callback": {
        "description": "trace MCPE ResourcePackManager listener/callback path that should call onResourcesLoaded",
        "events": [
            (0x006C0164, "MinecraftClient::init.entry"),
            (0x006C3148, "MinecraftClient::setupRenderer.entry"),
            (0x006C4856, "MinecraftClient::update.store-23c"),
            (0x006BEB1C, "MinecraftClient::onResourcesLoaded.entry"),
            (0x006BEFCE, "MinecraftClient::onResourcesLoaded.store-23e"),
            (0x009CC8F2, "GameRenderer::render.resource-ready-gate"),
            (0x00A13858, "ResourcePackManager::ctor.entry"),
            (0x00A1391C, "ResourcePackManager::ctor.copy-callback-source"),
            (0x00A13924, "ResourcePackManager::ctor.store-callback-fields"),
            (0x00A1392C, "ResourcePackManager::ctor.clone-callback"),
            (0x00A1397A, "ResourcePackManager::ctor.store-singleton"),
            (0x00A13A58, "ResourcePackManager::init.entry"),
            (0x00A13DDA, "ResourcePackManager::init.before-add-user-packs"),
            (0x00A13DE2, "ResourcePackManager::init.before-load-last-active-packs"),
            (0x00A1569C, "ResourcePackManager::registerListener.entry"),
            (0x00A156FE, "ResourcePackManager::registerListener.inserted-node"),
            (0x00A157D8, "ResourcePackManager::notifyActiveChanged.entry"),
            (0x00A157DE, "ResourcePackManager::notifyActiveChanged.dispatch-listener"),
            (0x00A157F0, "ResourcePackManager::setActiveResourcePacks.entry"),
            (0x00A15926, "ResourcePackManager::setActiveResourcePacks.dispatch-listener"),
            (0x00A16058, "ResourcePackManager::preloadTextures.entry"),
            (0x00A1606E, "ResourcePackManager::preloadTextures.mark-preloading"),
            (0x00A1623A, "ResourcePackManager::preloadTextures.loop-next-pack"),
            (0x00A1636C, "ResourcePackManager::preloadTextures.before-worker-queue"),
            (0x00A16378, "ResourcePackManager::preloadTextures.worker-queue-call"),
            (0x00A1637C, "ResourcePackManager::preloadTextures.after-worker-queue"),
            (0x00A163D8, "ResourcePackManager::preloadTextures.cleanup-work-fn"),
            (0x00A1645A, "ResourcePackManager::preloadTextures.cleanup-done-fn"),
            (0x00A166E6, "ResourcePackManager::preloadTextures.exception-cleanup-work-fn"),
            (0x00A166F2, "ResourcePackManager::preloadTextures.exception-cleanup-work-fn-call"),
            (0x00A16778, "ResourcePackManager::preloadTextures.exception-cleanup-done-fn"),
            (0x00A16782, "ResourcePackManager::preloadTextures.exception-cleanup-done-fn-call"),
            (0x00A16982, "ResourcePackManager::preloadTextures.return"),
            (0x00AF6A0C, "BackgroundWorker::queue.entry"),
            (0x00AF6A24, "BackgroundWorker::queue.store-job"),
            (0x00AF6AFE, "BackgroundWorker::queue.enqueued"),
            (0x00AF6B02, "BackgroundWorker::queue.signal"),
            (0x00AF6B74, "BackgroundWorker::_processNextCallback.entry"),
            (0x00AF6B8E, "BackgroundWorker::_processNextCallback.invoke"),
            (0x00AF6D0C, "BackgroundWorker::_processCallbacks.entry"),
            (0x00AF6D68, "BackgroundWorker::processNext.entry"),
            (0x00AF6D8A, "BackgroundWorker::processNext.invoke-work"),
            (0x00AF6D90, "BackgroundWorker::processNext.after-work"),
            (0x00AF6DCC, "BackgroundWorker::_processNextCoroutine.entry"),
            (0x00AF6DE4, "BackgroundWorker::_processNextCoroutine.invoke-work"),
            (0x00AF6DEA, "BackgroundWorker::_processNextCoroutine.after-work"),
            (0x00AF7294, "BackgroundWorker::flush.entry"),
            (0x00AF72D4, "BackgroundWorker::sync.entry"),
            (0x00AF8834, "WorkerPool::_createWorker.entry"),
            (0x00AF88B8, "WorkerPool::_start.entry"),
            (0x00AF88DA, "WorkerPool::_start.create-main-worker-call"),
            (0x00AF8906, "WorkerPool::_start.store-main-worker"),
            (0x00AF8D90, "WorkerPool::start.entry"),
            (0x00AF8E40, "WorkerPool::_runCoroutines.entry"),
            (0x00AF8EA2, "WorkerPool::_runCoroutines.before-worker-process"),
            (0x00AF8EAA, "WorkerPool::_runCoroutines.after-worker-process"),
            (0x00AF9050, "WorkerPool::processCoroutines.entry"),
            (0x00AF90C6, "WorkerPool::processCoroutines.run-coroutines-call"),
            (0x00AF90CA, "WorkerPool::processCoroutines.after-run-coroutines"),
            (0x00AF92B0, "WorkerPool::getInstance.entry"),
        ],
        "mem32": [
            (0x006C0164, "r0+0x238,+0x23c,+0x23e,+0x240"),
            (0x006C3148, "r0+0x238,+0x23c,+0x23e,+0x240"),
            (0x006C4856, "r4+0x238,+0x23c,+0x23e,+0x240"),
            (0x006BEB1C, "r0+0x238,+0x23c,+0x23e,+0x240"),
            (0x006BEFCE, "r8+0x238,+0x23c,+0x23e,+0x240"),
            (0x009CC8F2, "r7+0x238,+0x23c,+0x23e,+0x240"),
            (0x00A1391C, "r1+0,+0x4,+0x8,+0xc"),
            (0x00A13924, "r4+0x2c,+0x30,+0x38,+0x3c"),
            (0x00A1392C, "r4+0x2c,+0x30,+0x38,+0x3c"),
            (0x00A1397A, "r4+0x10,+0x14,+0x30,+0x38,+0x3c"),
            (0x00A13A58, "r0+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68"),
            (0x00A13DDA, "r4+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68"),
            (0x00A13DE2, "r4+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68"),
            (0x00A1569C, "r0+0x8,+0xc,+0x10,+0x14"),
            (0x00A1569C, "r1+0"),
            (0x00A156FE, "r3+0,+0x4"),
            (0x00A157DE, "r4+0,+0x4"),
            (0x00A157F0, "r0+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68"),
            (0x00A157F0, "r1+0,+0x4,+0x8"),
            (0x00A15926, "r4+0,+0x4"),
            (0x00A16058, "r0+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68,+0x6c"),
            (0x00A1606E, "r5+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68,+0x6c"),
            (0x00A1623A, "r5+0,+0x4,+0x8,+0xc"),
            (0x00A1636C, "sp+0x44,+0x48,+0x4c,+0x5c,+0x60,+0x64,+0x7c,+0x80,+0x84"),
            (0x00A1636C, "r10+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
            (0x00A16378, "sp+0x5c,+0x60,+0x64,+0x7c,+0x80,+0x84"),
            (0x00A16378, "r1+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
            (0x00A1637C, "r10+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68,+0x6c"),
            (0x00A163D8, "sp+0x5c,+0x60,+0x64"),
            (0x00A1645A, "sp+0x7c,+0x80,+0x84"),
            (0x00A166E6, "sp+0x5c,+0x60,+0x64"),
            (0x00A16778, "sp+0x7c,+0x80,+0x84"),
            (0x00A16982, "sp+0x138,+0x13c"),
            (0x00AF6A0C, "r0+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60,+0x80,+0x88"),
            (0x00AF6A0C, "r1+0,+0x4,+0x8,+0xc"),
            (0x00AF6A0C, "r2+0,+0x4,+0x8,+0xc"),
            (0x00AF6A24, "sp+0,+0x4,+0x8,+0xc,+0x20,+0x24"),
            (0x00AF6AFE, "r6+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60,+0x80,+0x88"),
            (0x00AF6B74, "r0+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
            (0x00AF6B8E, "sp+0,+0x4,+0x8,+0xc"),
            (0x00AF6D68, "r0+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
            (0x00AF6D8A, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6D90, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6DCC, "r0+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6DE4, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6DEA, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF88B8, "r0+0,+0x4,+0x8,+0xc,+0x20,+0x24,+0x28,+0x50,+0x54,+0x58,+0xd8,+0xdc"),
            (0x00AF8906, "r0+0,+0x20,+0x24,+0x28,+0xd8,+0xdc"),
            (0x00AF8E40, "r0+0x50,+0x54,+0x58,+0x5c,+0x60,+0xd8,+0xdc"),
            (0x00AF8EA2, "r10+0x50,+0x54,+0x58,+0xd8,+0xdc"),
            (0x00AF9050, "r0+0x50,+0x54,+0x58,+0x5c,+0x60,+0x98,+0xa8,+0xd8,+0xdc"),
            (0x00AF90CA, "r4+0x50,+0x54,+0x58,+0x5c,+0x60,+0xd8,+0xdc"),
        ],
        "deref32": [
            (0x00A157DE, "r4+0x4,+0,+0x8"),
            (0x00A15926, "r4+0x4,+0,+0x8"),
            (0x00AF8EA2, "r7+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
        ],
        "bytes": [
            (0x006C4856, "r4+0x238,16"),
            (0x006BEFCE, "r8+0x238,16"),
            (0x009CC8F2, "r7+0x238,16"),
            (0x00A13DDA, "r4+0x30,16"),
            (0x00A13DE2, "r4+0x30,16"),
            (0x00A1636C, "sp+0x5c,48"),
            (0x00A1636C, "sp+0x7c,48"),
        ],
        "event_limit": 1600,
    },
    "resource-worker": {
        "description": "trace MCPE resource preload worker execution without hot queue-loop events",
        "events": [
            (0x006BEB1C, "MinecraftClient::onResourcesLoaded.entry"),
            (0x006BEFCE, "MinecraftClient::onResourcesLoaded.store-23e"),
            (0x009CC8F2, "GameRenderer::render.resource-ready-gate"),
            (0x00A1569C, "ResourcePackManager::registerListener.entry"),
            (0x00A156FE, "ResourcePackManager::registerListener.inserted-node"),
            (0x00A157D8, "ResourcePackManager::notifyActiveChanged.entry"),
            (0x00A157DE, "ResourcePackManager::notifyActiveChanged.dispatch-listener"),
            (0x00A157F0, "ResourcePackManager::setActiveResourcePacks.entry"),
            (0x00A15926, "ResourcePackManager::setActiveResourcePacks.dispatch-listener"),
            (0x00A16058, "ResourcePackManager::preloadTextures.entry"),
            (0x00A16378, "ResourcePackManager::preloadTextures.worker-queue-call"),
            (0x00A16982, "ResourcePackManager::preloadTextures.return"),
            (0x00A17318, "ResourcePackManager::preloadTextures.done-callback"),
            (0x00A1754C, "ResourcePackManager::preloadTextures.work-load-texture"),
            (0x00AF6B74, "BackgroundWorker::_processNextCallback.entry"),
            (0x00AF6B8E, "BackgroundWorker::_processNextCallback.invoke"),
            (0x00AF6D0C, "BackgroundWorker::_processCallbacks.entry"),
            (0x00AF6D68, "BackgroundWorker::processNext.entry"),
            (0x00AF6D8A, "BackgroundWorker::processNext.invoke-work"),
            (0x00AF6D90, "BackgroundWorker::processNext.after-work"),
            (0x00AF6DCC, "BackgroundWorker::_processNextCoroutine.entry"),
            (0x00AF6DE4, "BackgroundWorker::_processNextCoroutine.invoke-work"),
            (0x00AF6DEA, "BackgroundWorker::_processNextCoroutine.after-work"),
            (0x00AF8E40, "WorkerPool::_runCoroutines.entry"),
            (0x00AF8EA2, "WorkerPool::_runCoroutines.before-worker-process"),
            (0x00AF8EAA, "WorkerPool::_runCoroutines.after-worker-process"),
            (0x00AF9050, "WorkerPool::processCoroutines.entry"),
            (0x00AF90C6, "WorkerPool::processCoroutines.run-coroutines-call"),
            (0x00AF90CA, "WorkerPool::processCoroutines.after-run-coroutines"),
        ],
        "mem32": [
            (0x006BEFCE, "r8+0x238,+0x23c,+0x23e,+0x240"),
            (0x009CC8F2, "r7+0x238,+0x23c,+0x23e,+0x240"),
            (0x00A1569C, "r0+0x8,+0xc,+0x10,+0x14"),
            (0x00A1569C, "r1+0"),
            (0x00A156FE, "r3+0,+0x4"),
            (0x00A157DE, "r4+0,+0x4"),
            (0x00A157F0, "r0+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68,+0x6c"),
            (0x00A15926, "r4+0,+0x4"),
            (0x00A16058, "r0+0x10,+0x14,+0x30,+0x38,+0x3c,+0x48,+0x4c,+0x64,+0x68,+0x6c"),
            (0x00A16378, "sp+0x5c,+0x60,+0x64,+0x7c,+0x80,+0x84"),
            (0x00A16378, "r1+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60,+0x80,+0x88"),
            (0x00A17318, "r0+0,+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x00A1754C, "r0+0,+0x4,+0x8,+0xc"),
            (0x00AF6B74, "r0+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
            (0x00AF6B8E, "sp+0,+0x4,+0x8,+0xc"),
            (0x00AF6D68, "r0+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6D8A, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6D90, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6DCC, "r0+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6DE4, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF6DEA, "r4+0,+0x18,+0x1c,+0x20,+0x24,+0x28,+0x30,+0x40,+0x48"),
            (0x00AF8E40, "r0+0x50,+0x54,+0x58,+0x5c,+0x60,+0xd8,+0xdc"),
            (0x00AF8EA2, "r10+0x50,+0x54,+0x58,+0xd8,+0xdc"),
            (0x00AF9050, "r0+0x50,+0x54,+0x58,+0x5c,+0x60,+0x98,+0xa8,+0xd8,+0xdc"),
            (0x00AF90CA, "r4+0x50,+0x54,+0x58,+0x5c,+0x60,+0xd8,+0xdc"),
        ],
        "deref32": [
            (0x00A157DE, "r4+0x4,+0,+0x8"),
            (0x00A15926, "r4+0x4,+0,+0x8"),
            (0x00AF8EA2, "r7+0,+0x18,+0x1c,+0x40,+0x44,+0x48,+0x5c,+0x60"),
        ],
        "bytes": [
            (0x006BEFCE, "r8+0x238,16"),
            (0x009CC8F2, "r7+0x238,16"),
            (0x00A16378, "sp+0x5c,48"),
            (0x00A16378, "sp+0x7c,48"),
        ],
        "event_limit": 3000,
    },
    "resource-done": {
        "description": "trace ResourcePackManager preload done-callback counter and final resource-loaded callback",
        "events": [
            (0x006BEB1C, "MinecraftClient::onResourcesLoaded.entry"),
            (0x006BEFCE, "MinecraftClient::onResourcesLoaded.store-23e"),
            (0x009CC8F2, "GameRenderer::render.resource-ready-gate"),
            (0x00A17318, "ResourcePackManager::preloadTextures.done-callback.entry"),
            (0x00A17344, "ResourcePackManager::preloadTextures.done-callback.load-count"),
            (0x00A1734A, "ResourcePackManager::preloadTextures.done-callback.check-count"),
            (0x00A17350, "ResourcePackManager::preloadTextures.done-callback.final-callback-load"),
            (0x00A1735E, "ResourcePackManager::preloadTextures.done-callback.final-callback-call"),
            (0x00A1754C, "ResourcePackManager::preloadTextures.work-load-texture.entry"),
        ],
        "mem32": [
            (0x006BEB1C, "r0+0x238,+0x23c,+0x23e,+0x240"),
            (0x006BEFCE, "r8+0x238,+0x23c,+0x23e,+0x240"),
            (0x009CC8F2, "r7+0x238,+0x23c,+0x23e,+0x240"),
            (0x00A17318, "r0+0,+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x00A17344, "r0+0"),
            (0x00A1734A, "r4+0x30,+0x38,+0x3c,+0x60"),
            (0x00A17350, "r4+0x30,+0x38,+0x3c,+0x60"),
            (0x00A1735E, "r4+0x30,+0x38,+0x3c,+0x60"),
            (0x00A1754C, "r0+0,+0x4,+0x8,+0xc"),
        ],
        "bytes": [
            (0x006BEFCE, "r8+0x238,16"),
            (0x009CC8F2, "r7+0x238,16"),
        ],
        "event_limit": 5000,
    },
    "localization": {
        "description": "trace Localization::_appendTranslations entry strings",
        "events": [
            (0x00A7B5B4, "Localization::_appendTranslations.entry"),
        ],
        "mem32": [
            (0x00A7B5B4, "r0+0,+0x4,+0x8,+0xc"),
            (0x00A7B5B4, "r1+0,+0x4,+0x8,+0xc"),
        ],
        "cxx_string": [
            (0x00A7B5B4, "r1+0,160"),
        ],
        "event_limit": 200,
    },
    "localization-hot": {
        "description": "trace Localization::_appendTranslations loop PCs from first-visible-draw profile",
        "events": [
            (0x00A7B5B4, "Localization::_appendTranslations.entry"),
            (0x00A7B71C, "Localization::_appendTranslations.loop-branch"),
            (0x00A7B72A, "Localization::_appendTranslations.hot-alu"),
            (0x00A7B72E, "Localization::_appendTranslations.hot-it"),
            (0x00A7B730, "Localization::_appendTranslations.hot-alu2"),
            (0x00A7B87C, "Localization::_appendTranslations.hot-load"),
            (0x00A7B882, "Localization::_appendTranslations.hot-branch"),
        ],
        "mem32": [
            (0x00A7B5B4, "r0+0,+0x4,+0x8,+0xc"),
            (0x00A7B5B4, "r1+0,+0x4,+0x8,+0xc"),
        ],
        "cxx_string": [
            (0x00A7B5B4, "r1+0,160"),
        ],
        "event_limit": 800,
    },
    "webtoken": {
        "description": "trace MCPE certificate WebToken creation without HLE-ing game logic",
        "events": [
            (0x006AFD50, "Certificate::createBasicCertificate.copy-token-call"),
            (0x006AE900, "WebToken::copy.entry"),
            (0x006B2A40, "WebToken::createFromData.entry"),
            (0x006B2A7E, "WebToken::createFromData.after-token-builder"),
            (0x006B2A8C, "WebToken::createFromData.check-token-builder"),
            (0x006B2BDE, "WebToken::createFromData.check-signature-compare"),
            (0x006B2C24, "WebToken::createFromData.return-null-token-builder"),
            (0x006B2C2C, "WebToken::createFromData.return-null-signature"),
            (0x006B2C7C, "WebToken::createFromData.return-success"),
        ],
        "mem32": [
            (0x006AFD50, "sp+0x5c,+0x60,+0xe0,+0x12c"),
            (0x006AE900, "sp+0,+0x4,+0x8,+0xc,+0x10,+0x14,+0x18,+0x1c"),
            (0x006B2A40, "r0+0"),
            (0x006B2A40, "r1+0,+0x4,+0x8,+0xc,+0x10,+0x14,+0x18,+0x1c"),
            (0x006B2A40, "r2+0,+0x4,+0x8,+0xc"),
            (0x006B2A7E, "sp+0x6c,+0x70,+0x74,+0x78"),
            (0x006B2A8C, "sp+0x6c,+0x70,+0x74,+0x78"),
            (0x006B2BDE, "sp+0x6c,+0x70,+0x74,+0x78"),
            (0x006B2C24, "r8+0"),
            (0x006B2C2C, "r8+0"),
            (0x006B2C7C, "r8+0"),
        ],
        "deref32": [
            (0x006B2A40, "r2+0x8,+0x4"),
        ],
        "event_limit": 200,
    },
    "keygen": {
        "description": "trace the MCPE PrivateKeyManager/OpenSSL key generation path",
        "events": [
            (0x006B0F26, "PrivateKeyManager::ctor.generate-call"),
            (0x011CD458, "Asymmetric::generateKeyPair.wrapper-entry"),
            (0x011CD45E, "Asymmetric::generateKeyPair.wrapper-jump"),
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x011CD9AC, "OpenSSLInterface::generateKeyPair.after-new-ctx"),
            (0x011CD9B8, "OpenSSLInterface::generateKeyPair.after-keygen-init"),
            (0x011CD9EA, "OpenSSLInterface::generateKeyPair.after-ec-curve-ctrl"),
            (0x011CD9F8, "OpenSSLInterface::generateKeyPair.after-paramgen"),
            (0x011CDA0C, "OpenSSLInterface::generateKeyPair.keygen-ctx-created"),
            (0x011CDA1A, "OpenSSLInterface::generateKeyPair.after-keygen-init2"),
            (0x011CDA28, "OpenSSLInterface::generateKeyPair.after-keygen2"),
            (0x011CDB34, "OpenSSLInterface::generateKeyPair.fail-keygen2"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
            (0x006B0F28, "PrivateKeyManager::ctor.generate-return"),
        ],
        "mem32": [
            (0x011CD458, "r0+0,+0x4,+0x8,+0xc,+0x10"),
            (0x011CD45E, "r0+0,+0x4,+0x8,+0xc,+0x10"),
        ],
        "cxx_string": [
            (0x006B0F28, "r4+0x4,128"),
            (0x006B0F28, "r4+0xc,128"),
        ],
        "event_limit": 200,
    },
    "keygen-ec": {
        "description": "trace bundled OpenSSL EC key generation and public-point multiply",
        "events": [
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x011CDA28, "OpenSSLInterface::generateKeyPair.after-keygen2"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
            (0x012399B8, "EC_KEY_generate_key.entry"),
            (0x01239A08, "EC_KEY_generate_key.after-private-rand"),
            (0x01239A4C, "EC_KEY_generate_key.private-ready"),
            (0x01239A98, "EC_KEY_generate_key.after-point-mul"),
            (0x01239AA4, "EC_KEY_generate_key.success-store"),
            (0x01239A40, "EC_KEY_generate_key.cleanup-return"),
            (0x01237240, "EC_POINT_mul.entry"),
            (0x01237280, "EC_POINT_mul.call-method"),
            (0x01237284, "EC_POINT_mul.after-method"),
            (0x012378EC, "ec_wNAF_mul.entry"),
            (0x01236CFC, "EC_POINT_is_on_curve.entry"),
            (0x01236D40, "EC_POINT_is_on_curve.after-method"),
            (0x012EC640, "ec_GFp_simple_is_on_curve.entry"),
            (0x012EC71C, "ec_GFp_simple_is_on_curve.return"),
        ],
        "mem32": [
            (0x012399B8, "r0+0x4,+0x8,+0xc"),
            (0x01239A08, "r0"),
            (0x01239A4C, "r4+0,+0x4"),
            (0x01239A98, "r0"),
            (0x01239AA4, "r6+0x8,+0xc"),
            (0x01237240, "r0+0"),
            (0x01237240, "r1+0"),
            (0x01237240, "r2+0"),
            (0x01237284, "r0"),
            (0x01236D40, "r0"),
            (0x012EC71C, "r0"),
        ],
        "event_limit": 300,
    },
    "keygen-mul": {
        "description": "trace EC_POINT_mul inputs and generated public-point coordinates",
        "events": [
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x012399B8, "EC_KEY_generate_key.entry"),
            (0x01239A4C, "EC_KEY_generate_key.private-ready"),
            (0x01237240, "EC_POINT_mul.entry"),
            (0x012378EC, "ec_wNAF_mul.entry"),
            (0x01237284, "EC_POINT_mul.after-method"),
            (0x01239A98, "EC_KEY_generate_key.after-point-mul"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
        ],
        "mem32": [
            (0x01237240, "r0+0,+0x4,+0x8,+0x1c,+0x48,+0x74,+0x88,+0xa0,+0xa4"),
            (0x01237240, "r1+0,+0x4,+0x8,+0x18,+0x1c,+0x2c,+0x30,+0x40"),
            (0x01237240, "r2+0,+0x4,+0x8,+0xc,+0x10"),
            (0x01237284, "r0"),
            (0x01239A98, "r0"),
            (0x01239A98, "r9+0,+0x4,+0x8,+0x18,+0x1c,+0x2c,+0x30,+0x40"),
        ],
        "deref32": [
            (0x01237240, "r0+0x4,+0x4,+0"),
            (0x01237240, "r0+0x4,+0x18,+0"),
            (0x01237240, "r0+0x4,+0x2c,+0"),
        ],
        "bytes": [
            (0x01237240, "*r0+0x4,+0x4,64"),
            (0x01237240, "*r0+0x4,+0x18,64"),
            (0x01237240, "*r0+0x4,+0x2c,64"),
            (0x01237240, "*r2+0,64"),
            (0x01239A98, "*r9+0x4,64"),
            (0x01239A98, "*r9+0x18,64"),
            (0x01239A98, "*r9+0x2c,64"),
            (0x01239A98, "*r4+0,64"),
        ],
        "event_limit": 200,
    },
    "bn-mont": {
        "description": "trace OpenSSL BN Montgomery multiplication inputs and outputs",
        "events": [
            (0x012A014C, "BN_mod_mul_montgomery.entry"),
            (0x012E0620, "bn_mul_mont.entry"),
            (0x012A0234, "BN_mod_mul_montgomery.after-bn_mul_mont"),
            (0x012A0274, "BN_mod_mul_montgomery.fast-return"),
        ],
        "mem32": [
            (0x012A014C, "r0+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012A014C, "r1+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012A014C, "r2+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012A014C, "r3+0x18,+0x1c,+0x40"),
            (0x012E0620, "sp+0,+0x4"),
            (0x012A0234, "r9+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012A0274, "r9+0,+0x4,+0x8,+0xc,+0x10"),
        ],
        "bytes": [
            (0x012A014C, "*r1+0,64"),
            (0x012A014C, "*r2+0,64"),
            (0x012A014C, "*r3+0x18,64"),
            (0x012E0620, "r0,64"),
            (0x012E0620, "r1,64"),
            (0x012E0620, "r2,64"),
            (0x012E0620, "r3,64"),
            (0x012E0620, "*sp+0,16"),
            (0x012A0234, "*r9+0,64"),
            (0x012A0274, "*r9+0,64"),
        ],
        "event_limit": 80,
    },
    "bn-mod-sqr": {
        "description": "trace OpenSSL BN_mod_sqr inputs, BN_sqr output, and reduced output",
        "events": [
            (0x0129CF1C, "BN_mod_sqr.entry"),
            (0x0129CF38, "BN_mod_sqr.after-bn-sqr"),
            (0x0129CF54, "BN_mod_sqr.before-bn-div"),
            (0x0129CF58, "BN_mod_sqr.return"),
        ],
        "mem32": [
            (0x0129CF1C, "r0+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CF1C, "r1+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CF1C, "r2+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CF38, "r0"),
            (0x0129CF38, "r6+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CF54, "r6+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CF58, "r6+0,+0x4,+0x8,+0xc,+0x10"),
        ],
        "bytes": [
            (0x0129CF1C, "*r1+0,128"),
            (0x0129CF1C, "*r2+0,128"),
            (0x0129CF38, "*r6+0,128"),
            (0x0129CF54, "*r6+0,128"),
            (0x0129CF58, "*r6+0,128"),
        ],
        "event_limit": 240,
    },
    "bn-nnmod": {
        "description": "trace OpenSSL BN_nnmod inputs, BN_div remainder, and final non-negative remainder",
        "events": [
            (0x0129CD18, "BN_nnmod.entry"),
            (0x0129CD40, "BN_nnmod.after-bn-div"),
            (0x0129CD78, "BN_nnmod.before-corrective-add"),
            (0x0129CD7C, "BN_nnmod.return"),
        ],
        "mem32": [
            (0x0129CD18, "r0+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CD18, "r1+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CD18, "r2+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CD40, "r0"),
            (0x0129CD40, "r4+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CD78, "r4+0,+0x4,+0x8,+0xc,+0x10"),
            (0x0129CD7C, "r4+0,+0x4,+0x8,+0xc,+0x10"),
        ],
        "bytes": [
            (0x0129CD18, "*r1+0,192"),
            (0x0129CD18, "*r2+0,192"),
            (0x0129CD40, "*r4+0,192"),
            (0x0129CD78, "*r4+0,192"),
            (0x0129CD7C, "*r4+0,192"),
        ],
        "event_limit": 320,
    },
    "bn-div-words": {
        "description": "trace OpenSSL bn_div_words quotient estimates backed by __aeabi_uldivmod",
        "events": [
            (0x0123468C, "bn_div_words.entry"),
            (0x012346B0, "bn_div_words.return"),
        ],
        "event_limit": 1000,
    },
    "bn-div": {
        "description": "trace OpenSSL BN_div inputs and final remainder normalization",
        "events": [
            (0x012991A0, "BN_div.entry"),
            (0x012996C4, "BN_div.before-rem-rshift"),
            (0x012996E4, "BN_div.after-rem-rshift"),
        ],
        "mem32": [
            (0x012991A0, "r1+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012991A0, "r2+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012991A0, "r3+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012996C4, "sp+0x28"),
            (0x012996C4, "r7+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012996E4, "r6+0,+0x4,+0x8,+0xc,+0x10"),
        ],
        "bytes": [
            (0x012991A0, "*r2+0,192"),
            (0x012991A0, "*r3+0,192"),
            (0x012996C4, "*r7+0,192"),
            (0x012996E4, "*r6+0,192"),
        ],
        "event_limit": 360,
    },
    "bn-div-loop": {
        "description": "trace OpenSSL BN_div quotient-estimate and multiply/subtract loop",
        "events": [
            (0x0129954C, "BN_div.loop.uldiv-call"),
            (0x01299550, "BN_div.loop.uldiv-ret"),
            (0x01299570, "BN_div.loop.after-mls"),
            (0x01299584, "BN_div.loop.after-umull"),
            (0x012995A0, "BN_div.loop.after-mla"),
            (0x012995A4, "BN_div.loop.pre-product-cmp"),
            (0x012995B0, "BN_div.loop.quotient-correct1"),
            (0x012995F8, "BN_div.loop.ready-mul-sub"),
            (0x01299614, "BN_div.loop.bn-mul-call"),
            (0x01299618, "BN_div.loop.bn-mul-ret"),
            (0x01299640, "BN_div.loop.bn-sub-call"),
            (0x01299644, "BN_div.loop.bn-sub-ret"),
            (0x01299758, "BN_div.loop.bn-add-call"),
            (0x01299760, "BN_div.loop.bn-add-ret"),
        ],
        "bytes": [
            (0x01299614, "r1,64"),
            (0x01299618, "*r5+0,64"),
            (0x01299640, "r1,64"),
            (0x01299640, "r2,64"),
            (0x01299644, "*sp+0x7c,64"),
            (0x01299758, "r1,64"),
            (0x01299758, "r2,64"),
            (0x01299760, "*sp+0x7c,64"),
        ],
        "event_limit": 4000,
    },
    "font-texture-pair": {
        "description": "trace native Font::init TextureGroup lookups for bitmap font atlases",
        "events": [
            (0x0073CA50, "Font::init.entry"),
            (0x0073CA80, "Font::init.before-getTexturePair"),
            (0x0073CA88, "Font::init.after-getTexturePair"),
            (0x0073CAC4, "Font::init.before-texture-use"),
            (0x011F045C, "TextureGroup::getTexturePair.entry"),
        ],
        "cxx_string": [
            (0x0073CA80, "r1+0,96"),
            (0x0073CA80, "r1+4,96"),
            (0x011F045C, "r1+0,96"),
            (0x011F045C, "r1+4,96"),
        ],
        "deref32": [
            (0x0073CA50, "r0+0xa54"),
        ],
        "mem32": [
            (0x011F045C, "r0+0,+4,+8,+0x10,+0x14,+0x18,+0x1c"),
        ],
        "event_limit": 80,
    },
    "ec-point-ops": {
        "description": "trace OpenSSL EC point add/double/make-affine input and output coordinates",
        "events": [
            (0x012EB560, "ec_GFp_simple_dbl.entry"),
            (0x012EB63C, "ec_GFp_simple_dbl.return"),
            (0x012EC00C, "ec_GFp_simple_add.entry"),
            (0x012EC24C, "ec_GFp_simple_add.return"),
            (0x012EC930, "ec_GFp_simple_make_affine.entry"),
            (0x012EC954, "ec_GFp_simple_make_affine.return-fast"),
            (0x012EC9D8, "ec_GFp_simple_make_affine.return"),
        ],
        "mem32": [
            (0x012EB560, "r1+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EB560, "r2+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EB63C, "r10+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC00C, "r1+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC00C, "r2+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC00C, "r3+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC24C, "r8+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC930, "r1+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC954, "r6+0,+0x4,+0x18,+0x2c,+0x40"),
            (0x012EC9D8, "r6+0,+0x4,+0x18,+0x2c,+0x40"),
        ],
        "bytes": [
            (0x012EB560, "*r2+0x4,64"),
            (0x012EB560, "*r2+0x18,64"),
            (0x012EB560, "*r2+0x2c,64"),
            (0x012EB63C, "*r10+0x4,64"),
            (0x012EB63C, "*r10+0x18,64"),
            (0x012EB63C, "*r10+0x2c,64"),
            (0x012EC00C, "*r2+0x4,64"),
            (0x012EC00C, "*r2+0x18,64"),
            (0x012EC00C, "*r2+0x2c,64"),
            (0x012EC00C, "*r3+0x4,64"),
            (0x012EC00C, "*r3+0x18,64"),
            (0x012EC00C, "*r3+0x2c,64"),
            (0x012EC24C, "*r8+0x4,64"),
            (0x012EC24C, "*r8+0x18,64"),
            (0x012EC24C, "*r8+0x2c,64"),
            (0x012EC930, "*r1+0x4,64"),
            (0x012EC930, "*r1+0x18,64"),
            (0x012EC930, "*r1+0x2c,64"),
            (0x012EC954, "*r6+0x4,64"),
            (0x012EC954, "*r6+0x18,64"),
            (0x012EC954, "*r6+0x2c,64"),
            (0x012EC9D8, "*r6+0x4,64"),
            (0x012EC9D8, "*r6+0x18,64"),
            (0x012EC9D8, "*r6+0x2c,64"),
        ],
        "event_limit": 400,
    },
    "point-affine": {
        "description": "trace OpenSSL EC Jacobian-to-affine conversion internals",
        "events": [
            (0x012EB210, "ec_GFp_simple_point_get_affine.entry"),
            (0x012EB2A4, "ec_GFp_simple_point_get_affine.after-z-decode"),
            (0x012EB2E0, "ec_GFp_simple_point_get_affine.after-z-invert"),
            (0x012EB30C, "ec_GFp_simple_point_get_affine.after-zinv-sqr"),
            (0x012EB340, "ec_GFp_simple_point_get_affine.after-x"),
            (0x012EB37C, "ec_GFp_simple_point_get_affine.after-y-prep"),
            (0x012EB4A4, "ec_GFp_simple_point_get_affine.after-y"),
            (0x012EB3A8, "ec_GFp_simple_point_get_affine.return"),
        ],
        "mem32": [
            (0x012EB210, "r1+0,+0x4,+0x8,+0x18,+0x1c,+0x2c,+0x30,+0x40"),
            (0x012EB210, "r2+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012EB210, "r3+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012EB2A4, "r10+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012EB2E0, "r8+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012EB30C, "r9+0,+0x4,+0x8,+0xc,+0x10"),
            (0x012EB340, "sp+0xc,+0x10"),
            (0x012EB37C, "sp+0xc,+0x10,+0x14"),
            (0x012EB4A4, "sp+0xc,+0x10,+0x14"),
            (0x012EB3A8, "sp+0xc,+0x10,+0x14"),
        ],
        "bytes": [
            (0x012EB210, "*r1+0x4,64"),
            (0x012EB210, "*r1+0x18,64"),
            (0x012EB210, "*r1+0x2c,64"),
            (0x012EB2A4, "*r10+0,64"),
            (0x012EB2E0, "*r8+0,64"),
            (0x012EB30C, "*r9+0,64"),
            (0x012EB340, "*sp+0xc,+0,64"),
            (0x012EB37C, "*sp+0x14,+0,64"),
            (0x012EB4A4, "*sp+0x10,+0,64"),
            (0x012EB3A8, "*sp+0xc,+0,64"),
            (0x012EB3A8, "*sp+0x10,+0,64"),
        ],
        "event_limit": 600,
    },
    "keygen-serialize": {
        "description": "trace bundled OpenSSL EC private-key DER serialization and point2oct output",
        "events": [
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
            (0x01255684, "i2d_PrivateKey.entry"),
            (0x012556C0, "i2d_PrivateKey.call-ec"),
            (0x012556D4, "i2d_PrivateKey.return"),
            (0x012A254C, "i2d_ECPrivateKey.entry"),
            (0x012A2594, "i2d_ECPrivateKey.after-struct-new"),
            (0x012A2688, "i2d_ECPrivateKey.after-private-octet"),
            (0x012A26D4, "i2d_ECPrivateKey.after-point2oct-size"),
            (0x012A2728, "i2d_ECPrivateKey.after-point2oct-write"),
            (0x012A2914, "i2d_ECPrivateKey.have-public-bit-string"),
            (0x012A2938, "i2d_ECPrivateKey.after-public-bit-string"),
            (0x012A27E4, "i2d_ECPrivateKey.after-asn1-write"),
            (0x0123A3F0, "EC_POINT_point2oct.entry"),
            (0x0123A4B8, "EC_POINT_point2oct.call-method"),
            (0x012A81B0, "ec_GFp_simple_point2oct.entry"),
            (0x012A828C, "ec_GFp_simple_point2oct.have-output-len"),
            (0x012A82F0, "ec_GFp_simple_point2oct.after-affine"),
            (0x012A8330, "ec_GFp_simple_point2oct.after-form-byte"),
            (0x012A8378, "ec_GFp_simple_point2oct.after-x-bytes"),
            (0x012A8420, "ec_GFp_simple_point2oct.after-y-bytes"),
            (0x012A8470, "ec_GFp_simple_point2oct.return-uncompressed"),
            (0x012A8490, "ec_GFp_simple_point2oct.return-cleanup"),
        ],
        "mem32": [
            (0x01255684, "r0+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012556D4, "r0"),
            (0x012A254C, "r0+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012A2688, "r0"),
            (0x012A26D4, "r0"),
            (0x012A2728, "r0"),
            (0x012A2938, "r0"),
            (0x0123A3F0, "r0+0"),
            (0x0123A3F0, "r1+0"),
            (0x012A81B0, "r0+0"),
            (0x012A81B0, "r1+0"),
            (0x012A81B0, "r1+0x4,+0x8,+0x18,+0x1c,+0x2c,+0x30,+0x40"),
            (0x012A828C, "r0"),
            (0x012A82F0, "r0"),
            (0x012A82F0, "sp+0x8,+0xc"),
            (0x012A8378, "sp+0x8,+0xc"),
            (0x012A8420, "sp+0x8,+0xc"),
            (0x012A8470, "r6"),
            (0x012A8490, "r6"),
        ],
        "bytes": [
            (0x0123A3F0, "r3+0,100"),
            (0x012A81B0, "r3+0,100"),
            (0x012A81B0, "*r1+0x4,64"),
            (0x012A81B0, "*r1+0x18,64"),
            (0x012A81B0, "*r1+0x2c,64"),
            (0x012A82F0, "*sp+0x8,+0,64"),
            (0x012A82F0, "*sp+0xc,+0,64"),
            (0x012A8378, "r5+0,100"),
            (0x012A8420, "r5+0,100"),
            (0x012A8470, "r5+0,100"),
            (0x012A8490, "r5+0,100"),
        ],
        "event_limit": 300,
    },
    "signdata": {
        "description": "trace MCPE PrivateKeyManager/OpenSSL signing after key generation succeeds",
        "events": [
            (0x006B11A0, "PrivateKeyManager::sign.entry"),
            (0x006B11BA, "PrivateKeyManager::sign.virtual-call"),
            (0x011CDD28, "OpenSSLInterface::signData.entry"),
            (0x011CDD54, "OpenSSLInterface::signData.after-d2i-private"),
            (0x011CDD62, "OpenSSLInterface::signData.after-ctx-new"),
            (0x011CDD6E, "OpenSSLInterface::signData.after-sign-init"),
            (0x011CDD94, "OpenSSLInterface::signData.after-ec-curve-ctrl"),
            (0x011CDE28, "OpenSSLInterface::signData.after-ctx-ctrl"),
            (0x011CDE48, "OpenSSLInterface::signData.after-sign-size"),
            (0x011CDE68, "OpenSSLInterface::signData.after-sign-data"),
            (0x011CDE82, "OpenSSLInterface::signData.success"),
            (0x011CDDB2, "OpenSSLInterface::signData.fail-private"),
            (0x011CDDBE, "OpenSSLInterface::signData.fail-ctx"),
            (0x011CDDD0, "OpenSSLInterface::signData.fail-sign-init"),
            (0x011CDDE8, "OpenSSLInterface::signData.fail-ec-curve"),
            (0x011CDF00, "OpenSSLInterface::signData.fail-ctx-ctrl"),
            (0x011CDE96, "OpenSSLInterface::signData.fail-sign-size"),
            (0x011CDEAE, "OpenSSLInterface::signData.fail-sign-data"),
            (0x006B11BC, "PrivateKeyManager::sign.returned"),
        ],
        "mem32": [
            (0x011CDE48, "sp+0x14"),
            (0x011CDE68, "sp+0x14"),
            (0x011CDE82, "r9+0"),
            (0x006B11BC, "r0+0"),
        ],
        "deref32": [
            (0x006B11A0, "r1+0x8,+0,+0x14"),
        ],
        "cxx_string": [
            (0x006B11A0, "r2+0,768"),
            (0x011CDD28, "r2+0,192"),
            (0x011CDD28, "r3+0,768"),
            (0x006B11BC, "r0+0,256"),
        ],
        "bytes": [
            (0x011CDD28, "*r2+0,167"),
        ],
        "event_limit": 250,
    },
    "d2i-private": {
        "description": "trace bundled OpenSSL d2i_AutoPrivateKey and EC private-key decode",
        "events": [
            (0x0125555C, "d2i_AutoPrivateKey.entry"),
            (0x01255584, "d2i_AutoPrivateKey.after-sequence-any"),
            (0x0125558C, "d2i_AutoPrivateKey.after-type-num-1"),
            (0x0125559C, "d2i_AutoPrivateKey.after-type-num-2"),
            (0x012555AC, "d2i_AutoPrivateKey.after-type-num-3"),
            (0x012555B8, "d2i_AutoPrivateKey.selected-keytype"),
            (0x012555D8, "d2i_AutoPrivateKey.after-privatekey-decode"),
            (0x01255604, "d2i_AutoPrivateKey.after-pkcs8-decode"),
            (0x01255618, "d2i_AutoPrivateKey.after-pkcs8-convert"),
            (0x01255648, "d2i_AutoPrivateKey.fail-pkcs8-convert"),
            (0x01255670, "d2i_AutoPrivateKey.success-no-output-arg"),
            (0x012A2170, "d2i_ECPrivateKey.entry"),
            (0x012A2184, "d2i_ECPrivateKey.after-asn1-item-d2i"),
            (0x012A21A0, "d2i_ECPrivateKey.have-output-key"),
            (0x012A21CC, "d2i_ECPrivateKey.check-parameters-type"),
            (0x012A2214, "d2i_ECPrivateKey.after-parameters"),
            (0x012A2220, "d2i_ECPrivateKey.have-group"),
            (0x012A223C, "d2i_ECPrivateKey.after-private-key-bn"),
            (0x012A2260, "d2i_ECPrivateKey.after-public-point-new"),
            (0x012A22A8, "d2i_ECPrivateKey.after-public-point-decode"),
            (0x012A22B0, "d2i_ECPrivateKey.success"),
            (0x012A22C4, "d2i_ECPrivateKey.return"),
            (0x012A231C, "d2i_ECPrivateKey.fail-missing-public-key"),
            (0x012A2364, "d2i_ECPrivateKey.fail-missing-private-key"),
            (0x012A2388, "d2i_ECPrivateKey.fail-public-point-decode"),
            (0x012A23C8, "d2i_ECPrivateKey.fail-missing-group"),
            (0x012A23EC, "d2i_ECPrivateKey.fail-private-key-bn"),
            (0x012A2410, "d2i_ECPrivateKey.fail-public-point-new"),
            (0x012A2434, "d2i_ECPrivateKey.derive-public-from-private"),
            (0x012A244C, "d2i_ECPrivateKey.after-public-derive"),
            (0x012A2460, "d2i_ECPrivateKey.fail-public-derive"),
            (0x012A2484, "d2i_ECPrivateKey.fail-asn1-item-d2i"),
            (0x0123A4D4, "EC_POINT_oct2point.entry"),
            (0x0123A594, "EC_POINT_oct2point.call-method"),
            (0x0123A5A0, "EC_POINT_oct2point.after-method"),
            (0x012A8520, "ec_GFp_simple_oct2point.entry"),
            (0x012A8550, "ec_GFp_simple_oct2point.after-form-parse"),
            (0x012A8624, "ec_GFp_simple_oct2point.after-length-check"),
            (0x012A86BC, "ec_GFp_simple_oct2point.fail-length"),
            (0x012A866C, "ec_GFp_simple_oct2point.after-x-bin2bn"),
            (0x012A8684, "ec_GFp_simple_oct2point.after-x-range"),
            (0x012A87AC, "ec_GFp_simple_oct2point.after-y-bin2bn"),
            (0x012A87C4, "ec_GFp_simple_oct2point.after-y-range"),
            (0x012A8870, "ec_GFp_simple_oct2point.after-set-affine"),
            (0x012A873C, "ec_GFp_simple_oct2point.after-is-on-curve"),
            (0x012A8748, "ec_GFp_simple_oct2point.fail-not-on-curve"),
            (0x012A8768, "ec_GFp_simple_oct2point.fail"),
            (0x012A876C, "ec_GFp_simple_oct2point.cleanup"),
        ],
        "mem32": [
            (0x0125555C, "r1+0"),
            (0x01255584, "sp+0xc"),
            (0x012555B8, "r8"),
            (0x012555D8, "r0"),
            (0x01255604, "r0"),
            (0x01255618, "r5"),
            (0x012A2170, "r1+0"),
            (0x012A2184, "r0"),
            (0x012A21A0, "r4+0,+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012A21CC, "r7+0,+0x4"),
            (0x012A2214, "r7+0"),
            (0x012A2220, "r5+0,+0x4,+0x8,+0xc"),
            (0x012A223C, "r0"),
            (0x012A2260, "r0"),
            (0x012A22A8, "r0"),
            (0x012A22B0, "r4+0,+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012A22C4, "r4"),
            (0x012A244C, "r0"),
            (0x0123A4D4, "r0+0"),
            (0x0123A4D4, "r1+0"),
            (0x0123A594, "r2+0"),
            (0x0123A5A0, "r0"),
            (0x012A8520, "r0+0"),
            (0x012A8520, "r1+0"),
            (0x012A8520, "r2+0"),
            (0x012A8550, "r2"),
            (0x012A8624, "r2"),
            (0x012A866C, "r0"),
            (0x012A8684, "r0"),
            (0x012A87AC, "r0"),
            (0x012A87C4, "r0"),
            (0x012A8870, "r0"),
            (0x012A873C, "r0"),
        ],
        "bytes": [
            (0x0125555C, "*r1+0,167"),
            (0x012A2170, "*r1+0,167"),
            (0x0123A4D4, "r2+0,100"),
            (0x012A8520, "r2+0,100"),
        ],
        "event_limit": 300,
    },
}

OBJECT_RE = re.compile(
    r"^\s+(?P<name>[^:]+): load_bias (?P<load_bias>0x[0-9a-fA-F]+), "
    r"mapped (?P<memory_base>0x[0-9a-fA-F]+)\+(?P<memory_size>0x[0-9a-fA-F]+),"
)
CRASH_RE = re.compile(
    r"address (?P<fault>0x[0-9a-fA-F]+) is not mapped .* "
    r"while executing (?P<isa>Arm|Thumb) at (?P<pc>0x[0-9a-fA-F]+)"
)
RECENT_PC_RE = re.compile(
    r"^\s+(?P<isa>Arm|Thumb) pc=(?P<pc>0x[0-9a-fA-F]+).* "
    r"sp=(?P<sp>0x[0-9a-fA-F]+) lr=(?P<lr>0x[0-9a-fA-F]+)$"
)
LABEL_RE = re.compile(r"^\s*([0-9a-fA-F]+) <([^>]+)>:")
FIRST_SWAP_RE = re.compile(r"native activity reached eglSwapBuffers at step (?P<step>\d+)")
LIVE_FRAME_RE = re.compile(
    r"^sdl2-live: frame=(?P<frame>\d+) "
    r"events=(?P<events>\d+) "
    r"payload=(?P<payload>\d+) "
    r"draws arrays=(?P<draw_arrays>\d+) "
    r"elements=(?P<draw_elements>\d+) "
    r"skipped_client_attrib=(?P<skipped_client_attrib>\d+) "
    r"skipped_missing_indices=(?P<skipped_missing_indices>\d+) "
    r"readback=(?P<readback_width>\d+)x(?P<readback_height>\d+) "
    r"rgb=(?P<readback_rgb>\d+) "
    r"alpha=(?P<readback_alpha>\d+) "
    r"gl_errors=(?P<gl_errors>\d+)"
)
LIVE_FRAME_LIMIT_RE = re.compile(
    r"^sdl2-live: reached frame limit "
    r"frames=(?P<frames>\d+) "
    r"events=(?P<events>\d+) "
    r"payload=(?P<payload>\d+)"
)
LIVE_DRAW_ELEMENTS_LIMIT_RE = re.compile(
    r"^sdl2-live: reached draw-elements limit "
    r"draw_elements=(?P<draw_elements>\d+) "
    r"frames=(?P<frames>\d+) "
    r"events=(?P<events>\d+) "
    r"payload=(?P<payload>\d+)"
    r"(?: readback=(?P<readback_width>\d+)x(?P<readback_height>\d+) "
    r"rgb=(?P<readback_rgb>\d+) "
    r"alpha=(?P<readback_alpha>\d+) "
    r"gl_errors=(?P<gl_errors>\d+))?"
)
LIVE_STOP_SCREENSHOT_RE = re.compile(
    r"^sdl2-live: stop screenshot "
    r"path=(?P<path>.+?) "
    r"width=(?P<width>\d+) "
    r"height=(?P<height>\d+) "
    r"bytes=(?P<bytes>\d+)"
)


STAGE_MARKERS = [
    ("constructors", "native constructors completed"),
    ("fmod_jni", "launch: libfmod.so JNI_OnLoad"),
    ("mcpe_jni", "launch: libminecraftpe.so JNI_OnLoad"),
    ("native_register_this", "launch: nativeRegisterThis"),
    ("activity_on_create", "launch: ANativeActivity_onCreate"),
    ("android_main", "launch: android_main"),
    ("first_swap", "native activity reached eglSwapBuffers"),
    ("completed", "native activity launch returned"),
    ("completed", "sdl2-live: reached frame limit"),
    ("completed", "sdl2-live: reached draw-elements limit"),
]


def parse_u32(raw: str) -> int:
    return int(raw, 16 if raw.lower().startswith("0x") else 10)


def run_capture(cmd, *, env=None, timeout=60, log_path=None):
    started = time.time()
    timed_out = False
    try:
        completed = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
            env=env,
        )
        output = completed.stdout or ""
        returncode = completed.returncode
    except subprocess.TimeoutExpired as err:
        timed_out = True
        output = err.stdout or ""
        if isinstance(output, bytes):
            output = output.decode("utf-8", errors="replace")
        returncode = None

    elapsed = time.time() - started
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text(output, encoding="utf-8")
    return {
        "cmd": cmd,
        "returncode": returncode,
        "timed_out": timed_out,
        "elapsed_seconds": round(elapsed, 3),
        "output": output,
    }


def unique_trace_dir(base: pathlib.Path) -> pathlib.Path:
    stamp = int(time.time())
    for idx in range(100):
        candidate = base.parent / f"{base.name}-{stamp}" if idx == 0 else base.parent / f"{base.name}-{stamp}-{idx}"
        try:
            candidate.mkdir(parents=True)
            return candidate
        except FileExistsError:
            continue
    raise RuntimeError(f"could not create a unique trace directory under {base.parent}")


def prepare_trace_dir(args) -> pathlib.Path:
    if args.trace_dir:
        trace_dir = pathlib.Path(args.trace_dir)
        trace_dir.mkdir(parents=True, exist_ok=True)
        if any(trace_dir.iterdir()) and not args.allow_existing_trace_dir:
            raise RuntimeError(
                f"{trace_dir} is not empty; use --allow-existing-trace-dir or choose a new path"
            )
        return trace_dir
    return unique_trace_dir(pathlib.Path(args.out_dir))


def parse_link_objects(link_log: str):
    objects = []
    for line in link_log.splitlines():
        match = OBJECT_RE.match(line)
        if not match:
            continue
        objects.append(
            {
                "name": match.group("name"),
                "load_bias": parse_u32(match.group("load_bias")),
                "memory_base": parse_u32(match.group("memory_base")),
                "memory_size": parse_u32(match.group("memory_size")),
            }
        )
    return objects


def object_by_name(objects, name: str):
    for obj in objects:
        if obj["name"] == name:
            return obj
    return None


def runtime_pc(objects, library_name: str, offset: int) -> int:
    obj = object_by_name(objects, library_name)
    if obj is None:
        raise RuntimeError(f"{library_name} was not linked; cannot resolve native trace preset")
    return obj["load_bias"] + offset


def trace_spec_for_offset(objects, offset: int, suffix: str, *, library_name: str = MCPE_LIBRARY) -> str:
    return f"0x{runtime_pc(objects, library_name, offset):08x}:{suffix}"


def append_native_trace_preset(config, preset_name: str, objects):
    preset = MCPE_NATIVE_TRACE_PRESETS[preset_name]
    for offset, name in preset["events"]:
        config["events"].append(trace_spec_for_offset(objects, offset, name))
    for offset, fields in preset.get("mem32", []):
        config["mem32"].append(trace_spec_for_offset(objects, offset, fields))
    for offset, fields in preset.get("deref32", []):
        config["deref32"].append(trace_spec_for_offset(objects, offset, fields))
    for offset, fields in preset.get("cxx_string", []):
        config["cxx_string"].append(trace_spec_for_offset(objects, offset, fields))
    for offset, fields in preset.get("bytes", []):
        config["bytes"].append(trace_spec_for_offset(objects, offset, fields))
    config["presets"].append(
        {
            "name": preset_name,
            "description": preset["description"],
            "library": MCPE_LIBRARY,
            "event_count": len(preset["events"]),
        }
    )
    config["event_limit"] = max(config["event_limit"] or 0, preset.get("event_limit", 0)) or None


def build_native_trace_config(args, objects):
    config = {
        "presets": [],
        "events": list(args.native_event or []),
        "mem32": list(args.native_event_mem32 or []),
        "deref32": list(args.native_event_deref32 or []),
        "cxx_string": list(args.native_event_cxx_string or []),
        "bytes": list(args.native_event_bytes or []),
        "event_limit": args.native_event_limit,
    }
    for preset_name in args.native_trace_preset or []:
        append_native_trace_preset(config, preset_name, objects)
    return config


def apply_native_trace_env(env, trace_dir: pathlib.Path, config):
    if not config["events"]:
        return
    env["AEMU_TRACE_NATIVE_EVENTS_JSONL"] = str(trace_dir / "native_events.jsonl")
    env["AEMU_TRACE_NATIVE_EVENTS"] = ";".join(config["events"])
    if config["mem32"]:
        env["AEMU_TRACE_NATIVE_EVENT_MEM32"] = ";".join(config["mem32"])
    if config["deref32"]:
        env["AEMU_TRACE_NATIVE_EVENT_DEREF32"] = ";".join(config["deref32"])
    if config["cxx_string"]:
        env["AEMU_TRACE_NATIVE_EVENT_CXX_STRING"] = ";".join(config["cxx_string"])
    if config["bytes"]:
        env["AEMU_TRACE_NATIVE_EVENT_BYTES"] = ";".join(config["bytes"])
    if config["event_limit"] is not None:
        env["AEMU_TRACE_NATIVE_EVENTS_LIMIT"] = str(config["event_limit"])


def parse_run_log(run_log: str):
    reached_stage = None
    for stage, marker in STAGE_MARKERS:
        if marker in run_log:
            reached_stage = stage

    first_swap_step = None
    match = FIRST_SWAP_RE.search(run_log)
    if match:
        first_swap_step = int(match.group("step"))

    crash = None
    match = CRASH_RE.search(run_log)
    if match:
        crash = {
            "fault_address": parse_u32(match.group("fault")),
            "isa": match.group("isa"),
            "pc": parse_u32(match.group("pc")),
        }

    recent = []
    live_frames = []
    frame_limit = None
    draw_elements_limit = None
    stop_screenshot = None
    for line in run_log.splitlines():
        match = RECENT_PC_RE.match(line)
        if match:
            recent.append(
                {
                    "isa": match.group("isa"),
                    "pc": parse_u32(match.group("pc")),
                    "sp": parse_u32(match.group("sp")),
                    "lr": parse_u32(match.group("lr")),
                }
            )
            continue
        match = LIVE_FRAME_RE.match(line)
        if match:
            live_frames.append({key: int(value) for key, value in match.groupdict().items()})
            continue
        match = LIVE_FRAME_LIMIT_RE.match(line)
        if match:
            frame_limit = {key: int(value) for key, value in match.groupdict().items()}
            continue
        match = LIVE_DRAW_ELEMENTS_LIMIT_RE.match(line)
        if match:
            draw_elements_limit = {
                key: int(value)
                for key, value in match.groupdict().items()
                if value is not None
            }
            continue
        match = LIVE_STOP_SCREENSHOT_RE.match(line)
        if match:
            stop_screenshot = {
                "path": match.group("path"),
                "width": int(match.group("width")),
                "height": int(match.group("height")),
                "bytes": int(match.group("bytes")),
            }

    live = {
        "logged_frame_count": len(live_frames),
        "max_logged_frame": max((frame["frame"] for frame in live_frames), default=0),
        "max_logged_draw_elements": max(
            (frame["draw_elements"] for frame in live_frames), default=0
        ),
        "max_logged_draw_arrays": max((frame["draw_arrays"] for frame in live_frames), default=0),
        "max_logged_readback_rgb": max(
            (frame["readback_rgb"] for frame in live_frames), default=0
        ),
        "max_logged_gl_errors": max((frame["gl_errors"] for frame in live_frames), default=0),
        "last_logged_frame": live_frames[-1] if live_frames else None,
        "frame_limit": frame_limit,
        "draw_elements_limit": draw_elements_limit,
        "stop_screenshot": stop_screenshot,
    }

    return {
        "reached_stage": reached_stage,
        "first_swap_step": first_swap_step,
        "live": live,
        "native_run_failed": "native run failed:" in run_log,
        "crash": crash,
        "recent_guest_pcs": recent,
        "hle_trace_count": sum(
            1 for line in run_log.splitlines() if line.startswith("HLE function=")
        ),
        "hle_file_trace_count": sum(
            1 for line in run_log.splitlines() if line.startswith("HLE file ")
        ),
    }


def extracted_so_path(apk: pathlib.Path, abi: str, library_name: str) -> pathlib.Path | None:
    if apk.suffix != ".apk":
        return None
    extracted = apk.with_suffix("")
    path = extracted / "lib" / abi / library_name
    return path if path.exists() else None


def symbolicate_pc(pc: int, isa: str | None, objects, apk: pathlib.Path, abi: str):
    selected = None
    for obj in objects:
        base = obj["memory_base"]
        end = base + obj["memory_size"]
        if base <= pc < end:
            selected = obj
            break
    if selected is None:
        return None

    offset = pc - selected["load_bias"]
    result = {
        "object": selected["name"],
        "load_bias": selected["load_bias"],
        "offset": offset,
    }
    so_path = extracted_so_path(apk, abi, selected["name"])
    if so_path is None:
        return result
    result["so_path"] = str(so_path)

    objdump = shutil.which("llvm-objdump")
    if objdump is None:
        return result

    start = max(0, offset - 0x20) & ~1
    stop = offset + 0x60
    cmd = [objdump, "-d", f"--start-address=0x{start:x}", f"--stop-address=0x{stop:x}", str(so_path)]
    if isa == "Thumb":
        cmd.insert(2, "--triple=thumbv7-none-linux-gnueabi")
    try:
        completed = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=10,
        )
    except subprocess.TimeoutExpired:
        result["disassembly_error"] = "llvm-objdump timed out"
        return result

    result["disassembly_returncode"] = completed.returncode
    result["disassembly"] = completed.stdout.splitlines()
    nearest = None
    for line in completed.stdout.splitlines():
        match = LABEL_RE.match(line)
        if not match:
            continue
        label_addr = int(match.group(1), 16)
        if label_addr <= offset:
            nearest = {
                "address": label_addr,
                "name": match.group(2),
                "delta": offset - label_addr,
            }
    if nearest is not None:
        result["nearest_symbol"] = nearest
    return result


def count_jsonl(path: pathlib.Path) -> int:
    if not path.exists():
        return 0
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def summarize_gles_events(path: pathlib.Path):
    summary = {
        "rows": 0,
        "kind_counts": {},
        "first_draw_index": None,
        "last_event_index": None,
    }
    if not path.exists():
        return summary
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            summary["rows"] += 1
            kind = row.get("kind") or "<unknown>"
            summary["kind_counts"][kind] = summary["kind_counts"].get(kind, 0) + 1
            index = row.get("index")
            if isinstance(index, int):
                summary["last_event_index"] = index
                if kind in ("DrawElements", "DrawArrays") and summary["first_draw_index"] is None:
                    summary["first_draw_index"] = index
    return summary


def summarize_pc_profile(path: pathlib.Path):
    summary = {
        "jsonl": str(path),
        "rows": 0,
        "samples": 0,
        "guest_instructions": 0,
        "unique_buckets": 0,
        "top": [],
    }
    if not path.exists():
        return summary
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            summary["rows"] += 1
            summary["samples"] = row.get("samples", summary["samples"])
            summary["guest_instructions"] = row.get(
                "guest_instructions", summary["guest_instructions"]
            )
            summary["unique_buckets"] = row.get("unique_buckets", summary["unique_buckets"])
            top = row.get("top")
            if isinstance(top, list):
                summary["top"] = top[:10]
    return summary


def collect_artifacts(trace_dir: pathlib.Path):
    draw_dir = trace_dir / "sdl-draw"
    gles_path = trace_dir / "gles_events.jsonl"
    native_events_path = trace_dir / "native_events.jsonl"
    pc_profile_path = trace_dir / "pc_profile.jsonl"
    stop_screenshot_path = trace_dir / "stop.png"
    gles = summarize_gles_events(gles_path)
    pc_profile = summarize_pc_profile(pc_profile_path)
    kind_counts = gles["kind_counts"]
    return {
        "gles_events_jsonl": str(gles_path),
        "gles_event_count": gles["rows"],
        "gles_kind_counts": kind_counts,
        "gles_swap_count": kind_counts.get("SwapBuffers", 0),
        "gles_draw_arrays_count": kind_counts.get("DrawArrays", 0),
        "gles_draw_elements_count": kind_counts.get("DrawElements", 0),
        "gles_first_draw_index": gles["first_draw_index"],
        "gles_last_event_index": gles["last_event_index"],
        "native_events_jsonl": str(native_events_path),
        "native_event_count": count_jsonl(native_events_path),
        "pc_profile_jsonl": str(pc_profile_path),
        "pc_profile_rows": pc_profile["rows"],
        "pc_profile_samples": pc_profile["samples"],
        "pc_profile_guest_instructions": pc_profile["guest_instructions"],
        "pc_profile_unique_buckets": pc_profile["unique_buckets"],
        "pc_profile_top": pc_profile["top"],
        "sdl_draw_dir": str(draw_dir),
        "sdl_draw_png_count": len(list(draw_dir.glob("*.png"))) if draw_dir.exists() else 0,
        "sdl_draw_manifest_count": count_jsonl(draw_dir / "draw_manifest.jsonl"),
        "stop_screenshot_png": str(stop_screenshot_path),
        "stop_screenshot_exists": stop_screenshot_path.exists(),
        "stop_screenshot_bytes": (
            stop_screenshot_path.stat().st_size if stop_screenshot_path.exists() else 0
        ),
    }


def native_event_matches(row: dict, needle: str) -> bool:
    needle = needle.lower()
    event = row.get("event")
    if isinstance(event, str) and needle in event.lower():
        return True
    pc = row.get("pc")
    return isinstance(pc, int) and needle in f"0x{pc:08x}".lower()


def read_jsonl(path: pathlib.Path):
    if not path.exists():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            rows.append(json.loads(line))
    return rows


def validate_expectations(args, summary):
    errors = []
    if args.expect_crash_pc is not None:
        crash = summary["run"].get("crash")
        actual = None if crash is None else crash["pc"]
        expected = parse_u32(args.expect_crash_pc)
        if actual != expected:
            errors.append(f"expected crash pc 0x{expected:08x}, got {format_hex(actual)}")
    if args.expect_fault_address is not None:
        crash = summary["run"].get("crash")
        actual = None if crash is None else crash["fault_address"]
        expected = parse_u32(args.expect_fault_address)
        if actual != expected:
            errors.append(f"expected fault address 0x{expected:08x}, got {format_hex(actual)}")
    if args.expect_stage is not None and summary["run"].get("reached_stage") != args.expect_stage:
        errors.append(
            f"expected stage {args.expect_stage}, got {summary['run'].get('reached_stage')}"
        )
    if args.expect_exit == "zero" and summary["process"]["returncode"] != 0:
        errors.append(f"expected zero exit, got {summary['process']['returncode']}")
    if args.expect_exit == "nonzero" and summary["process"]["returncode"] == 0:
        errors.append("expected nonzero exit, got 0")
    if args.expect_native_event:
        native_events = read_jsonl(pathlib.Path(summary["artifacts"]["native_events_jsonl"]))
        for expected in args.expect_native_event:
            if not any(native_event_matches(row, expected) for row in native_events):
                errors.append(f"expected native event matching {expected!r}")
    if args.expect_run_log_contains:
        run_log = pathlib.Path(summary["run_log"]).read_text(encoding="utf-8", errors="replace")
        for expected in args.expect_run_log_contains:
            if expected not in run_log:
                errors.append(f"expected run log to contain {expected!r}")
    live = summary["run"].get("live") or {}
    artifacts = summary["artifacts"]
    if args.min_live_frame and live.get("max_logged_frame", 0) < args.min_live_frame:
        errors.append(
            f"expected logged live frame >= {args.min_live_frame}, "
            f"got {live.get('max_logged_frame', 0)}"
        )
    if args.min_gles_events and artifacts["gles_event_count"] < args.min_gles_events:
        errors.append(
            f"expected at least {args.min_gles_events} GLES events, "
            f"got {artifacts['gles_event_count']}"
        )
    if args.min_gles_swaps and artifacts["gles_swap_count"] < args.min_gles_swaps:
        errors.append(
            f"expected at least {args.min_gles_swaps} GLES SwapBuffers, "
            f"got {artifacts['gles_swap_count']}"
        )
    if args.min_gles_draw_elements and artifacts["gles_draw_elements_count"] < args.min_gles_draw_elements:
        errors.append(
            f"expected at least {args.min_gles_draw_elements} GLES DrawElements, "
            f"got {artifacts['gles_draw_elements_count']}"
        )
    if args.min_sdl_draw_pngs and artifacts["sdl_draw_png_count"] < args.min_sdl_draw_pngs:
        errors.append(
            f"expected at least {args.min_sdl_draw_pngs} SDL draw PNGs, "
            f"got {artifacts['sdl_draw_png_count']}"
        )
    live_stop = live.get("draw_elements_limit") or {}
    max_readback_rgb = max(
        live.get("max_logged_readback_rgb", 0) or 0,
        live_stop.get("readback_rgb", 0) or 0,
    )
    max_gl_errors = max(
        live.get("max_logged_gl_errors", 0) or 0,
        live_stop.get("gl_errors", 0) or 0,
    )
    if args.min_readback_rgb and max_readback_rgb < args.min_readback_rgb:
        errors.append(
            f"expected readback rgb >= {args.min_readback_rgb}, "
            f"got {max_readback_rgb}"
        )
    if args.max_gl_errors is not None and max_gl_errors > args.max_gl_errors:
        errors.append(
            f"expected GL errors <= {args.max_gl_errors}, "
            f"got {max_gl_errors}"
        )
    if args.require_stop_screenshot and not artifacts["stop_screenshot_exists"]:
        errors.append("expected stop screenshot artifact")
    if (
        args.min_stop_screenshot_bytes
        and artifacts["stop_screenshot_bytes"] < args.min_stop_screenshot_bytes
    ):
        errors.append(
            f"expected stop screenshot >= {args.min_stop_screenshot_bytes} bytes, "
            f"got {artifacts['stop_screenshot_bytes']}"
        )
    if args.min_pc_profile_samples and artifacts["pc_profile_samples"] < args.min_pc_profile_samples:
        errors.append(
            f"expected at least {args.min_pc_profile_samples} PC profile samples, "
            f"got {artifacts['pc_profile_samples']}"
        )
    return errors


def has_expectation_gates(args) -> bool:
    return bool(
        args.expect_crash_pc
        or args.expect_fault_address
        or args.expect_stage
        or args.expect_native_event
        or args.expect_run_log_contains
        or args.min_live_frame
        or args.min_gles_events
        or args.min_gles_swaps
        or args.min_gles_draw_elements
        or args.min_sdl_draw_pngs
        or args.min_readback_rgb
        or args.require_stop_screenshot
        or args.min_stop_screenshot_bytes
        or args.min_pc_profile_samples
        or args.max_gl_errors is not None
    )


def format_hex(value):
    return "None" if value is None else f"0x{value:08x}"


def print_summary(summary, expectation_errors):
    crash = summary["run"].get("crash")
    symbolication = summary["run"].get("symbolication")
    print(f"trace_dir: {summary['trace_dir']}")
    print(
        "process: "
        f"returncode={summary['process']['returncode']} "
        f"timed_out={summary['process']['timed_out']} "
        f"elapsed={summary['process']['elapsed_seconds']}s"
    )
    print(f"stage: {summary['run'].get('reached_stage')}")
    live = summary["run"].get("live") or {}
    if live.get("logged_frame_count"):
        print(
            "live: "
            f"first_swap_step={summary['run'].get('first_swap_step')} "
            f"logged_frames={live.get('logged_frame_count')} "
            f"max_frame={live.get('max_logged_frame')} "
            f"max_logged_draw_elements={live.get('max_logged_draw_elements')} "
            f"max_rgb={live.get('max_logged_readback_rgb')} "
            f"max_gl_errors={live.get('max_logged_gl_errors')}"
        )
    if live.get("draw_elements_limit"):
        draw_limit = live["draw_elements_limit"]
        print(
            "live_stop: "
            f"draw_elements={draw_limit.get('draw_elements')} "
            f"frames={draw_limit.get('frames')} "
            f"events={draw_limit.get('events')} "
            f"payload={draw_limit.get('payload')} "
            f"rgb={draw_limit.get('readback_rgb')}"
        )
    if live.get("stop_screenshot"):
        screenshot = live["stop_screenshot"]
        print(
            "stop_screenshot: "
            f"path={screenshot.get('path')} "
            f"size={screenshot.get('width')}x{screenshot.get('height')} "
            f"bytes={screenshot.get('bytes')}"
        )
    if crash:
        print(
            "crash: "
            f"isa={crash['isa']} pc=0x{crash['pc']:08x} "
            f"fault=0x{crash['fault_address']:08x}"
        )
    if symbolication:
        nearest = symbolication.get("nearest_symbol") or {}
        symbol = nearest.get("name", "?")
        delta = nearest.get("delta")
        delta_text = "" if delta is None else f"+0x{delta:x}"
        print(
            "symbolication: "
            f"{symbolication.get('object')}+0x{symbolication.get('offset', 0):08x} "
            f"{symbol}{delta_text}"
        )
    native_trace = summary.get("native_trace") or {}
    if native_trace.get("presets") or native_trace.get("events"):
        presets = ",".join(preset["name"] for preset in native_trace.get("presets", [])) or "manual"
        print(
            "native_trace: "
            f"presets={presets} events={len(native_trace.get('events', []))} "
            f"limit={native_trace.get('event_limit')}"
        )
    hle_trace = summary.get("hle_trace") or {}
    if hle_trace.get("filter") or hle_trace.get("file_trace"):
        print(
            "hle_trace: "
            f"filter={hle_trace.get('filter')} "
            f"limit={hle_trace.get('limit')} "
            f"calls={summary['run'].get('hle_trace_count', 0)} "
            f"file_lines={summary['run'].get('hle_file_trace_count', 0)}"
        )
    artifacts = summary["artifacts"]
    print(
        "artifacts: "
        f"gles_events={artifacts['gles_event_count']} "
        f"gles_swaps={artifacts['gles_swap_count']} "
        f"gles_draw_elements={artifacts['gles_draw_elements_count']} "
        f"native_events={artifacts['native_event_count']} "
        f"pc_profile_samples={artifacts['pc_profile_samples']} "
        f"sdl_draw_pngs={artifacts['sdl_draw_png_count']} "
        f"sdl_draw_manifest_rows={artifacts['sdl_draw_manifest_count']}"
    )
    if artifacts["pc_profile_top"]:
        top = artifacts["pc_profile_top"][0]
        where = top.get("symbol") or top.get("library") or top.get("pc_hex")
        print(
            "pc_profile: "
            f"rows={artifacts['pc_profile_rows']} "
            f"unique={artifacts['pc_profile_unique_buckets']} "
            f"top={where}+{top.get('symbol_offset_hex', '0x0')} "
            f"count={top.get('count')} thread={top.get('thread_id')}"
        )
    print(f"run_log: {summary['run_log']}")
    print(f"summary_json: {summary['summary_json']}")
    if expectation_errors:
        print("expectations: failed")
        for error in expectation_errors:
            print(f"  {error}")
    else:
        print("expectations: ok")


def build_arg_parser():
    parser = argparse.ArgumentParser(
        description="Run the default MCPE SDL2 smoke path and write deterministic trace artifacts."
    )
    parser.add_argument("--apk", default=str(DEFAULT_APK))
    parser.add_argument("--abi", default=DEFAULT_ABI)
    parser.add_argument("--binary", default=str(DEFAULT_BINARY))
    parser.add_argument("--cpu-backend", choices=["aemu", "dynarmic"], default="aemu")
    parser.add_argument(
        "--dynarmic-run-ticks",
        type=int,
        help="set AEMU_DYNARMIC_RUN_TICKS for chunked native Dynarmic runs",
    )
    parser.add_argument("--out-dir", default=str(DEFAULT_OUT_DIR))
    parser.add_argument("--trace-dir")
    parser.add_argument("--allow-existing-trace-dir", action="store_true")
    parser.add_argument("--steps", type=int, default=DEFAULT_STEPS)
    parser.add_argument("--frames", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--display", default=":0")
    parser.add_argument("--gles-event-limit", type=int, default=50000)
    parser.add_argument(
        "--gles-event-skip",
        type=int,
        help="skip GLES events before this global event index before applying match/limit",
    )
    parser.add_argument("--draw-dump-limit", type=int, default=10)
    parser.add_argument(
        "--first-visible-draw",
        action="store_true",
        help="apply the validated SDL2 milestone gates for the first visible DrawElements frame",
    )
    parser.add_argument(
        "--first-visible-draw-resource",
        action="store_true",
        help="run the first-visible-draw milestone with the resource-done native trace preset",
    )
    parser.add_argument(
        "--stop-after-gles-draw-elements",
        type=int,
        help="stop SDL2 live execution once replayed DrawElements reaches this count",
    )
    parser.add_argument(
        "--fake-time-step-nanos",
        type=int,
        help="set AEMU_FAKE_TIME_STEP_NANOS for Android time HLE diagnostics",
    )
    parser.add_argument(
        "--guest-thread-swap-slices",
        type=int,
        help="set AEMU_GUEST_THREAD_SWAP_SLICES for frame-boundary guest worker scheduling",
    )
    parser.add_argument(
        "--profile-pc",
        action="store_true",
        help="write low-overhead guest PC/function hot spot samples to pc_profile.jsonl",
    )
    parser.add_argument(
        "--profile-pc-interval",
        type=int,
        default=4096,
        help="sample one guest PC every N interpreted guest instructions",
    )
    parser.add_argument(
        "--profile-pc-limit",
        type=int,
        help="stop collecting PC samples after this many samples",
    )
    parser.add_argument(
        "--profile-pc-flush-interval",
        type=int,
        default=512,
        help="append a PC profile snapshot every N new samples",
    )
    parser.add_argument(
        "--profile-pc-top",
        type=int,
        default=80,
        help="include this many hottest buckets in each PC profile snapshot",
    )
    parser.add_argument(
        "--cpu-feature-preset",
        choices=["default", "no-neon"],
        default="default",
        help="override guest AT_HWCAP for CPU feature selection diagnostics",
    )
    parser.add_argument("--hwcap", help="set exact guest AT_HWCAP value, e.g. 0x8a0d7")
    parser.add_argument("--hwcap2", help="set exact guest AT_HWCAP2 value")
    parser.add_argument(
        "--native-trace-preset",
        action="append",
        choices=sorted(MCPE_NATIVE_TRACE_PRESETS),
        help="enable a built-in native PC trace preset using the linked object load bias",
    )
    parser.add_argument(
        "--native-event",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENTS spec, e.g. 0x70bb2a40:name",
    )
    parser.add_argument(
        "--native-event-mem32",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_MEM32 spec",
    )
    parser.add_argument(
        "--native-event-deref32",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_DEREF32 spec",
    )
    parser.add_argument(
        "--native-event-cxx-string",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_CXX_STRING spec",
    )
    parser.add_argument(
        "--native-event-bytes",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_BYTES spec, e.g. 0x716cdd28:*r2+0,192",
    )
    parser.add_argument("--native-event-limit", type=int)
    parser.add_argument(
        "--trace-hle",
        help="set AEMU_TRACE_HLE filter, e.g. '*' or '=open,=read,=fopen,=fread'",
    )
    parser.add_argument("--trace-hle-limit", type=int)
    parser.add_argument(
        "--trace-hle-file",
        action="store_true",
        help="enable AEMU_TRACE_HLE_FILE file/random/stdio diagnostics in run.log",
    )
    parser.add_argument("--expect-crash-pc")
    parser.add_argument("--expect-fault-address")
    parser.add_argument(
        "--expect-stage",
        choices=list(dict.fromkeys(stage for stage, _marker in STAGE_MARKERS)),
    )
    parser.add_argument("--expect-exit", choices=["any", "zero", "nonzero"], default="any")
    parser.add_argument(
        "--expect-native-event",
        action="append",
        help="require at least one structured native event whose name or PC contains this text",
    )
    parser.add_argument(
        "--expect-run-log-contains",
        action="append",
        help="require run.log to contain this exact substring",
    )
    parser.add_argument("--min-live-frame", type=int, default=0)
    parser.add_argument("--min-gles-events", type=int, default=0)
    parser.add_argument("--min-gles-swaps", type=int, default=0)
    parser.add_argument("--min-gles-draw-elements", type=int, default=0)
    parser.add_argument("--min-sdl-draw-pngs", type=int, default=0)
    parser.add_argument("--min-readback-rgb", type=int, default=0)
    parser.add_argument(
        "--require-stop-screenshot",
        action="store_true",
        help="require the draw-stop screenshot artifact written by --stop-after-gles-draw-elements",
    )
    parser.add_argument("--min-stop-screenshot-bytes", type=int, default=0)
    parser.add_argument("--min-pc-profile-samples", type=int, default=0)
    parser.add_argument("--max-gl-errors", type=int)
    parser.add_argument("--echo-log", action="store_true")
    return parser


def apply_milestone_defaults(args):
    if not (args.first_visible_draw or args.first_visible_draw_resource):
        return
    args.frames = max(args.frames, 260)
    args.timeout = max(args.timeout, 640 if args.first_visible_draw_resource else 560)
    if args.guest_thread_swap_slices is None:
        args.guest_thread_swap_slices = 256
    if args.stop_after_gles_draw_elements is None:
        args.stop_after_gles_draw_elements = 1
    args.min_gles_draw_elements = max(args.min_gles_draw_elements, 1)
    args.min_readback_rgb = max(args.min_readback_rgb, 1)
    args.require_stop_screenshot = True
    args.min_stop_screenshot_bytes = max(args.min_stop_screenshot_bytes, 1000)
    if args.max_gl_errors is None:
        args.max_gl_errors = 0
    if args.expect_stage is None:
        args.expect_stage = "completed"
    if args.expect_exit == "any":
        args.expect_exit = "zero"
    if args.first_visible_draw_resource:
        presets = list(args.native_trace_preset or [])
        if "resource-done" not in presets:
            presets.append("resource-done")
        args.native_trace_preset = presets


def main(argv=None):
    args = build_arg_parser().parse_args(argv)
    apply_milestone_defaults(args)
    apk = pathlib.Path(args.apk)
    binary = pathlib.Path(args.binary)
    if not apk.exists():
        raise SystemExit(f"APK not found: {apk}")
    if not binary.exists():
        raise SystemExit(f"aemu binary not found: {binary}; run cargo build --release --features sdl2")

    try:
        trace_dir = prepare_trace_dir(args)
    except RuntimeError as err:
        raise SystemExit(str(err)) from err

    link_log_path = trace_dir / "link.log"
    run_log_path = trace_dir / "run.log"
    summary_path = trace_dir / "summary.json"

    link = run_capture(
        [str(binary), "link-apk", str(apk), "--abi", args.abi, "--limit", "0"],
        timeout=30,
        log_path=link_log_path,
    )
    objects = parse_link_objects(link["output"])
    try:
        native_trace_config = build_native_trace_config(args, objects)
    except RuntimeError as err:
        raise SystemExit(str(err)) from err

    env = os.environ.copy()
    env.setdefault("DISPLAY", args.display)
    env.setdefault("SDL_VIDEO_X11_FORCE_EGL", "1")
    if args.cpu_feature_preset == "no-neon":
        env["AEMU_HWCAP"] = f"0x{ARMV7_NO_NEON_HWCAP:x}"
    if args.hwcap:
        env["AEMU_HWCAP"] = args.hwcap
    if args.hwcap2:
        env["AEMU_HWCAP2"] = args.hwcap2
    if args.fake_time_step_nanos is not None:
        env["AEMU_FAKE_TIME_STEP_NANOS"] = str(args.fake_time_step_nanos)
    if args.dynarmic_run_ticks is not None:
        env["AEMU_DYNARMIC_RUN_TICKS"] = str(args.dynarmic_run_ticks)
    if args.guest_thread_swap_slices is not None:
        env["AEMU_GUEST_THREAD_SWAP_SLICES"] = str(args.guest_thread_swap_slices)
    if args.profile_pc:
        env["AEMU_PROFILE_PC_JSONL"] = str(trace_dir / "pc_profile.jsonl")
        env["AEMU_PROFILE_PC_INTERVAL"] = str(args.profile_pc_interval)
        env["AEMU_PROFILE_PC_FLUSH_INTERVAL"] = str(args.profile_pc_flush_interval)
        env["AEMU_PROFILE_PC_TOP"] = str(args.profile_pc_top)
        if args.profile_pc_limit is not None:
            env["AEMU_PROFILE_PC_LIMIT"] = str(args.profile_pc_limit)
    env["AEMU_TRACE_GLES_EVENTS_JSONL"] = str(trace_dir / "gles_events.jsonl")
    env["AEMU_TRACE_GLES_EVENTS_MATCH"] = (
        "SwapBuffers,UseProgram,BindTexture,DrawElements,TexImage2D,TexSubImage2D"
    )
    env["AEMU_TRACE_GLES_EVENTS_LIMIT"] = str(args.gles_event_limit)
    if args.gles_event_skip is not None:
        env["AEMU_TRACE_GLES_EVENTS_SKIP"] = str(args.gles_event_skip)
    env["AEMU_TRACE_SDL_DRAW_CHANGES"] = "50"
    env["AEMU_DUMP_SDL_DRAW_CHANGES_DIR"] = str(trace_dir / "sdl-draw")
    env["AEMU_DUMP_SDL_DRAW_CHANGES_MATCH"] = "all"
    env["AEMU_DUMP_SDL_DRAW_CHANGES_LIMIT"] = str(args.draw_dump_limit)
    if args.trace_hle:
        env["AEMU_TRACE_HLE"] = args.trace_hle
    if args.trace_hle_limit is not None:
        env["AEMU_TRACE_HLE_LIMIT"] = str(args.trace_hle_limit)
    if args.trace_hle_file:
        env["AEMU_TRACE_HLE_FILE"] = "1"
    apply_native_trace_env(env, trace_dir, native_trace_config)

    cmd = [
        str(binary),
        "run-apk-native",
        str(apk),
        "--abi",
        args.abi,
        "--cpu-backend",
        args.cpu_backend,
        "--steps",
        str(args.steps),
        "--sdl2-live",
        "--sdl2-frames",
        str(args.frames),
    ]
    if args.stop_after_gles_draw_elements is not None:
        cmd.extend(
            [
                "--sdl2-stop-after-draw-elements",
                str(args.stop_after_gles_draw_elements),
                "--sdl2-stop-screenshot",
                str(trace_dir / "stop.png"),
            ]
        )
    run = run_capture(cmd, env=env, timeout=args.timeout, log_path=run_log_path)
    if args.echo_log and run["output"]:
        print(run["output"], end="")

    parsed_run = parse_run_log(run["output"])
    crash = parsed_run.get("crash")
    if crash is not None:
        parsed_run["symbolication"] = symbolicate_pc(
            crash["pc"], crash.get("isa"), objects, apk, args.abi
        )

    summary = {
        "trace_dir": str(trace_dir),
        "apk": str(apk),
        "abi": args.abi,
        "binary": str(binary),
        "cpu_backend": args.cpu_backend,
        "dynarmic_run_ticks": args.dynarmic_run_ticks,
        "link_log": str(link_log_path),
        "run_log": str(run_log_path),
        "summary_json": str(summary_path),
        "link": {
            "returncode": link["returncode"],
            "timed_out": link["timed_out"],
            "elapsed_seconds": link["elapsed_seconds"],
            "objects": objects,
        },
        "process": {
            "cmd": cmd,
            "returncode": run["returncode"],
            "timed_out": run["timed_out"],
            "elapsed_seconds": run["elapsed_seconds"],
        },
        "native_trace": native_trace_config,
        "hle_trace": {
            "filter": args.trace_hle,
            "limit": args.trace_hle_limit,
            "file_trace": args.trace_hle_file,
        },
        "time": {
            "fake_step_nanos": args.fake_time_step_nanos,
        },
        "threads": {
            "guest_thread_swap_slices": args.guest_thread_swap_slices,
        },
        "milestone": {
            "first_visible_draw": args.first_visible_draw,
            "first_visible_draw_resource": args.first_visible_draw_resource,
        },
        "pc_profile": {
            "enabled": args.profile_pc,
            "interval": args.profile_pc_interval if args.profile_pc else None,
            "limit": args.profile_pc_limit if args.profile_pc else None,
            "flush_interval": args.profile_pc_flush_interval if args.profile_pc else None,
            "top": args.profile_pc_top if args.profile_pc else None,
        },
        "run": parsed_run,
        "artifacts": collect_artifacts(trace_dir),
    }
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    expectation_errors = validate_expectations(args, summary)
    print_summary(summary, expectation_errors)

    if expectation_errors:
        return 1
    if args.expect_exit == "any" and not has_expectation_gates(args):
        return 0 if run["returncode"] == 0 else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
