#!/usr/bin/env python3
"""工具回调示例 — 展示 Python 函数作为 LLM 工具。"""
from kernel_client import KernelClient


# --- 工具定义 -----------------------------------------------------------------

def get_weather(city: str) -> str:
    """获取城市天气（示例：返回模拟数据）。"""
    weather = {
        "Beijing": "晴天，18°C，微风",
        "Shanghai": "多云，22°C，东南风",
        "Shenzhen": "阴天，25°C，南风",
    }
    return weather.get(city, f"{city} 天气数据暂不可用")


def calculator(expression: str) -> str:
    """安全计算数学表达式。"""
    try:
        # 仅允许数字和基本运算符
        allowed = set("0123456789+-*/(). ")
        if not set(expression).issubset(allowed):
            return "错误：表达式包含非法字符"
        result = eval(expression)
        return str(result)
    except Exception as e:
        return f"计算错误：{e}"


# --- 工具 Schema（告知 LLM 如何调用）-----------------------------------------

TOOLS = [
    {
        "name": "get_weather",
        "description": "获取指定城市的当前天气信息",
        "input_schema": {
            "type": "object",
            "properties": {
                "city": {"type": "string", "description": "城市名称（中文或英文）"}
            },
            "required": ["city"],
        },
    },
    {
        "name": "calculator",
        "description": "计算数学表达式",
        "input_schema": {
            "type": "object",
            "properties": {
                "expression": {"type": "string", "description": "数学表达式，如 '2 + 3 * 4'"}
            },
            "required": ["expression"],
        },
    },
]

# --- 工具回调字典 --------------------------------------------------------------

TOOL_HANDLERS = {
    "get_weather": get_weather,
    "calculator": calculator,
}


# --- 主程序 -------------------------------------------------------------------

def main():
    with KernelClient() as kernel:
        session = kernel.create_session(
            system_prompt="You are a helpful assistant with access to weather and calculator tools.",
            tools=TOOLS,
        )
        print(f"Session: {session}\n")

        questions = [
            "北京今天天气怎么样？",
            "帮我算一下 (123 + 456) * 2 等于多少？",
        ]

        for question in questions:
            print(f"User: {question}")
            print("Assistant: ", end="")
            for token in kernel.send_message(session, question, tools=TOOL_HANDLERS):
                print(token, end="", flush=True)
            print("\n")

        kernel.destroy_session(session)


if __name__ == "__main__":
    main()
