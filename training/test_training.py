import json
import tempfile
import unittest
from pathlib import Path
from train_lora import read_samples, encode_sample


class TrainingDataTests(unittest.TestCase):
    def test_feedback_requires_final_assistant_and_deduplicates(self):
        good = {"messages": [{"role": "user", "content": "你好"}, {"role": "assistant", "content": "你好呀"}]}
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "samples.jsonl"
            path.write_text(json.dumps(good) + "\n" + json.dumps(good), encoding="utf-8")
            self.assertEqual(len(read_samples(path)), 1)
            path.write_text(json.dumps({"messages": good["messages"][:-1]}), encoding="utf-8")
            with self.assertRaises(ValueError):
                read_samples(path)

    def test_only_approved_answer_has_labels_and_no_truncation(self):
        class Tokenizer:
            eos_token = "!"
            def apply_chat_template(self, *args, **kwargs):
                return "context"
            def encode(self, text, **kwargs):
                return list(range(len(text)))
        messages = [{"role": "user", "content": "u"}, {"role": "assistant", "content": "ok"}]
        row = encode_sample(Tokenizer(), messages, 10)
        self.assertEqual(row["labels"][:7], [-100] * 7)
        self.assertEqual(row["labels"][7:], [0, 1, 2])
        self.assertIsNone(encode_sample(Tokenizer(), messages, 9))


if __name__ == "__main__":
    unittest.main()
