import joblib
import pandas as pd
from rest_framework.response import Response
from rest_framework.views import APIView

from app.codec import decode_b64, decode_other, decode_signed, decode_verified


class StateView(APIView):
    def post(self, request):
        a = decode_b64(request.data["state"])
        b = decode_other(request.query_params["obj"])
        c = decode_signed(request.data["token"])
        d = decode_verified(request.data["blob"], request.headers["X-Signature"])
        return Response({"a": a, "b": b, "c": c, "d": d})


class ModelUpload(APIView):
    def post(self, request):
        upload = self.request.FILES["model"]
        model = joblib.load(upload)  # caught
        frame = pd.read_pickle(upload)  # caught
        return Response({"model": str(model), "rows": len(frame)})
