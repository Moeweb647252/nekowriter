import { afterEach, describe, expect, it, vi } from "vitest"

import {
  ApiError,
  getHealth,
  saveProjectSettings,
  saveSystemModelSettings,
  shutdownService,
} from "@/lib/api"

function jsonResponse(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe("失败的响应", () => {
  it("按错误信封给出错误码、文案与请求号", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse(409, {
          error: {
            code: "REVISION_CONFLICT",
            message: "设置已被其它窗口修改",
            details: { expected: "3" },
            request_id: "req-1",
          },
        })
      )
    )

    const failure = (await getHealth().catch((error: unknown) => error)) as ApiError
    expect(failure).toBeInstanceOf(ApiError)
    expect(failure.code).toBe("REVISION_CONFLICT")
    expect(failure.message).toBe("设置已被其它窗口修改")
    expect(failure.details).toEqual({ expected: "3" })
    expect(failure.requestId).toBe("req-1")
  })

  // 服务端换了形状（或代理返回了 HTML）时，界面必须仍拿到一句可展示的话与一个错误码，
  // 而不是 `undefined`；这类静默失败正是实测里最容易漏掉的东西。
  it("响应体不是错误信封时仍然给出可展示的失败", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("<!doctype html>", { status: 502 }))
    )

    const failure = (await getHealth().catch((error: unknown) => error)) as ApiError
    expect(failure).toBeInstanceOf(ApiError)
    expect(failure.code).toBe("HTTP_ERROR")
    expect(failure.message).toBe("本地服务返回 502")
    expect(failure.requestId).toBeNull()
  })

  it("成功响应按 JSON 解析返回", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse(200, { status: "ok" }))
    )

    await expect(getHealth()).resolves.toEqual({ status: "ok" })
  })
})

describe("连接丢失时的重试规则", () => {
  it("读请求不重试：一次失败就把原因抛给界面", async () => {
    const fetchMock = vi.fn(async () => {
      throw new TypeError("Failed to fetch")
    })
    vi.stubGlobal("fetch", fetchMock)

    await expect(getHealth()).rejects.toThrow("Failed to fetch")
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it("写请求重试一次，且请求体逐字相同：服务端因此按同一条回执重放", async () => {
    const inits: RequestInit[] = []
    const fetchMock = vi.fn(async (_path: string, init: RequestInit) => {
      inits.push(init)
      if (inits.length === 1) throw new TypeError("Failed to fetch")
      return jsonResponse(200, {})
    })
    vi.stubGlobal("fetch", fetchMock)

    await shutdownService()
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(inits[0].method).toBe("POST")
    expect(inits[0].body).toBe(inits[1].body)
  })
})

describe("命令请求体", () => {
  it("重试用同一个 commandId，而不是新生成一个", async () => {
    const bodies: Record<string, unknown>[] = []
    const fetchMock = vi.fn(async (_path: string, init: RequestInit) => {
      bodies.push(JSON.parse(String(init.body)) as Record<string, unknown>)
      if (bodies.length === 1) throw new TypeError("Failed to fetch")
      return jsonResponse(200, {})
    })
    vi.stubGlobal("fetch", fetchMock)

    await saveProjectSettings("project-1", {
      expectedRevision: "1",
      model: null,
      thinkingLevel: "off",
      auxiliaryToolBudget: "0",
    })
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(bodies[0].commandId).toEqual(expect.any(String))
    expect(bodies[1].commandId).toBe(bodies[0].commandId)
  })

  it("保存模型设置时省略密钥就是省略：请求体里没有 apiKey（保留语义由后端判定）", async () => {
    const fetchMock = vi.fn(async (_path: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as Record<string, unknown>
      expect(body.commandId).toEqual(expect.any(String))
      expect(body).not.toHaveProperty("apiKey")
      expect(body.modelId).toBe("writer-1")
      return jsonResponse(200, {})
    })
    vi.stubGlobal("fetch", fetchMock)

    await saveSystemModelSettings({
      expectedRevision: "1",
      apiBaseUrl: "https://model.example/v1",
      modelId: "writer-1",
      capabilities: {
        streaming: true,
        tools: true,
        streamUsage: true,
        thinking: true,
      },
      maxContextTokens: "32768",
      reservedOutputTokens: "4096",
    })
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })
})
