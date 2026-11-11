from huggingface_hub import hf_hub_download
from transformers import AutoModel, AutoTokenizer


def model():
    return AutoModel.from_pretrained("bert-base-uncased")


def tokenizer():
    return AutoTokenizer.from_pretrained("bert-base-uncased", revision="86b5e0934494bd15c9632b12f734a8a67f723594")


def weights():
    return hf_hub_download(repo_id="org/model", filename="model.safetensors")
