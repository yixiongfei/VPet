"""Train a local LoRA from explicit, reviewed VPet feedback. No upload or auto-training."""
import argparse
import json
from pathlib import Path


def read_samples(path):
    samples, seen = [], set()
    for number, line in enumerate(Path(path).read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        messages = row.get("messages")
        if not isinstance(messages, list) or len(messages) < 2:
            raise ValueError(f"Line {number}: missing conversation")
        for msg in messages:
            if msg.get("role") not in ("system", "user", "assistant") or not isinstance(msg.get("content"), str) or not msg["content"].strip():
                raise ValueError(f"Line {number}: invalid message")
        if messages[-1]["role"] != "assistant" or messages[-2]["role"] != "user":
            raise ValueError(f"Line {number}: must end in a user/approved assistant pair")
        clean = [{"role": m["role"], "content": m["content"]} for m in messages]
        key = json.dumps(clean, ensure_ascii=False, sort_keys=True)
        if key not in seen:
            seen.add(key)
            samples.append(clean)
    if not samples:
        raise ValueError("No approved samples. Rate or correct answers in VPet and export first.")
    return samples


def encode_sample(tokenizer, messages, max_length):
    # Mask ALL context; only the final explicitly approved reply receives training loss.
    prefix = tokenizer.apply_chat_template(messages[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False)
    suffix = messages[-1]["content"] + tokenizer.eos_token
    prompt_ids = tokenizer.encode(prefix, add_special_tokens=False)
    answer_ids = tokenizer.encode(suffix, add_special_tokens=False)
    # Do not silently train on truncated answers or remove the persona/user prompt.
    if len(prompt_ids) + len(answer_ids) > max_length:
        return None
    return {"input_ids": prompt_ids + answer_ids, "attention_mask": [1] * (len(prompt_ids) + len(answer_ids)), "labels": [-100] * len(prompt_ids) + answer_ids}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("data", type=Path)
    parser.add_argument("--base", default="Qwen/Qwen3.5-9B", help="Hugging Face base matching the Ollama model in use (qwen3:4b -> Qwen/Qwen3-4B)")
    parser.add_argument("--output", type=Path, default=Path("training/output/vpet-lora"))
    parser.add_argument("--validate-only", action="store_true", help="Validate JSONL only; no ML packages or model downloads needed")
    parser.add_argument("--allow-cpu", action="store_true", help="Explicitly opt into slow CPU training")
    parser.add_argument("--min-samples", type=int, default=30)
    parser.add_argument("--max-length", type=int, default=1024)
    parser.add_argument("--epochs", type=float, default=2)
    parser.add_argument("--merge", action="store_true", help="Also export merged Safetensors for Ollama import")
    args = parser.parse_args()
    samples = read_samples(args.data)
    print(f"Validated {len(samples)} unique approved conversations.")
    if args.validate_only:
        return
    if len(samples) < args.min_samples:
        raise ValueError(f"Need at least {args.min_samples} reviewed samples; found {len(samples)}. Collect more varied examples first.")
    if args.output.exists() and any(args.output.iterdir()):
        raise ValueError("Output already contains files. Choose a new --output to preserve previous training.")

    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer, DataCollatorForSeq2Seq, Trainer, TrainingArguments
    from peft import LoraConfig, get_peft_model

    gpu = torch.cuda.is_available()
    if not gpu and not args.allow_cpu:
        raise RuntimeError("No CUDA GPU. Inference works via Ollama. For LoRA use a CUDA machine, or explicitly pass --allow-cpu (slow and memory intensive).")
    tokenizer = AutoTokenizer.from_pretrained(args.base, trust_remote_code=False)
    tokenizer.pad_token = tokenizer.eos_token
    encoded = [encode_sample(tokenizer, m, args.max_length) for m in samples]
    data = [x for x in encoded if x is not None]
    print(f"Using {len(data)} samples; skipped {len(samples) - len(data)} over-length conversations.")
    if len(data) < args.min_samples:
        raise ValueError("Too few complete conversations within --max-length; raise the limit or curate shorter samples.")
    use_bf16 = gpu and torch.cuda.is_bf16_supported()
    dtype = torch.bfloat16 if use_bf16 else torch.float32
    model = AutoModelForCausalLM.from_pretrained(args.base, torch_dtype=dtype, trust_remote_code=False)
    model.config.use_cache = False
    model = get_peft_model(model, LoraConfig(r=8, lora_alpha=16, lora_dropout=0.05,
        target_modules=["q_proj", "v_proj"], task_type="CAUSAL_LM", bias="none"))
    model.enable_input_require_grads()
    model.print_trainable_parameters()
    arguments = TrainingArguments(output_dir=str(args.output), num_train_epochs=args.epochs,
        per_device_train_batch_size=1, gradient_accumulation_steps=8, learning_rate=1e-4,
        gradient_checkpointing=True, logging_steps=1, save_strategy="epoch", save_total_limit=2,
        bf16=use_bf16, use_cpu=not gpu, report_to="none", seed=42, optim="adamw_torch")
    trainer = Trainer(model=model, args=arguments, train_dataset=data,
        data_collator=DataCollatorForSeq2Seq(tokenizer, padding=True, label_pad_token_id=-100))
    trainer.train()
    model.save_pretrained(args.output)
    tokenizer.save_pretrained(args.output)
    if args.merge:
        merged = args.output / "merged"
        model.merge_and_unload().save_pretrained(merged, safe_serialization=True)
        tokenizer.save_pretrained(merged)
        (merged / "Modelfile").write_text("FROM .\nPARAMETER num_ctx 4096\n", encoding="utf-8")
        print(f"Merged model: {merged}. Import with ollama create vpet-personal -f {merged / 'Modelfile'}")
    print("Adapter saved. Compare with the base on held-out conversations before selecting it in VPet.")


if __name__ == "__main__":
    main()
