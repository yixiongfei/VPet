# 用你认可的回答训练她

日常聊天默认使用本机 Ollama 的 `qwen3.5:9b`（6.6 GB；`qwen3:4b` 也已安装，可在设置里切换）。人物设定会立即进入后续聊天的系统提示词；点赞或修改回答只收集训练样本，**不会偷偷训练或立刻改变模型权重**。

当前电脑为 32 GB 内存 + Intel Arc Pro；本地推理可以运行，CUDA 训练不可用。LoRA 训练脚本已准备，但没有真实样本时不应该训练。建议先收集至少 30 条、最好 100–300 条不同情境下的优质回答，并保留另外一批问题进行对比。

## 收集与检查

1. 在聊天框为满意的回答点「喜欢」，或者点「修改回答」写出理想答案。
2. 在设定中导出训练数据。仅导出明确认可或纠正过的模型回答，格式为每行一个 `{"messages":[...]}`；不导出失败、取消或纯差评答案。
3. 保存完整上下文和当时人物设定；聊天记录不会自动成为长期记忆。长期记忆仍使用「记住：…」等命令。
4. 数据文件保存在本机。检查并移除不想训练进去的私密内容，按情境检查重复样本和矛盾设定。

验证格式（只需 Python，无需下载训练依赖）：

```powershell
python training/train_lora.py "你的导出文件.jsonl" --validate-only
```

## 训练

建议在有 CUDA 的电脑上创建独立 Python 3.11/3.12 环境。首次会从 Hugging Face 下载原始基座权重（比 Ollama 量化模型大）；训练数据不上传。

```powershell
python -m venv training/.venv
training/.venv/Scripts/python -m pip install -r training/requirements.txt
training/.venv/Scripts/python training/train_lora.py "你的导出文件.jsonl" --base Qwen/Qwen3.5-9B --output training/output/run-001 --merge
```

LoRA 默认 rank 8、只训练 Q/V 投影、2 个 epoch、仅最后一条经过认可的回答计算 loss。超过上下文长度的样本会跳过，避免截断答案后训练。不会覆盖已有输出。`--allow-cpu` 可以明确选择 CPU 训练，但 9B 原始权重、梯度和激活需要很多内存，速度也慢；本机不默认启动这种长任务。

## 使用训练后的模型

`--merge` 会另外导出合并后的 Safetensors 和 Modelfile。使用与训练完全相同的基座与 tokenizer：

```powershell
.runtime/ollama/ollama.exe create vpet-personal -f training/output/run-001/merged/Modelfile
```

然后在 VPet 设定中把模型名改成 `vpet-personal`，检查模型连接并保存。先用没有参与训练的问题对比基座和新模型；若表现退步，切回 `qwen3:4b`。导出适配器本身不等于 Ollama 已加载它，必须先完成合并和导入。若当前 Ollama 不支持该 Safetensors 结构，可用 llama.cpp 转成 GGUF 再导入，见官方说明。

参考：[Qwen3.5-9B 模型](https://huggingface.co/Qwen/Qwen3.5-9B)（用 qwen3:4b 时改为 `--base Qwen/Qwen3-4B`）、[PEFT LoRA](https://huggingface.co/docs/peft/en/package_reference/lora)、[Ollama 模型导入](https://docs.ollama.com/import)。
