from transformers import AutoModel
AutoModel.from_pretrained('org/model')

from transformers import AutoModel
AutoModel.from_pretrained('org/model')  # nosec

from transformers import AutoModel
AutoModel.from_pretrained('org/model')  # nosec B615

from transformers import AutoModel
AutoModel.from_pretrained('org/model')  # nosec: B615

from transformers import AutoModel
AutoModel.from_pretrained('org/model')  # nosec huggingface_unsafe_download

from transformers import AutoModel
AutoModel.from_pretrained('org/model')  # nosec B101
