import subprocess

from django import forms
from rest_framework import serializers


class ToolSerializer(serializers.Serializer):
    tool = serializers.ChoiceField(choices=["ls", "df"])
    count = serializers.IntegerField()
    target = serializers.CharField()


class ToolForm(forms.Form):
    tool = forms.ChoiceField(choices=[("ls", "ls"), ("df", "df")])
    target = forms.CharField()


def run_tool(request):
    s = ToolSerializer(data=request.data)
    s.is_valid(raise_exception=True)
    # A fixed choice picks the program; a number reaches the shell.
    subprocess.run([s.validated_data["tool"], "-h"])
    subprocess.run(f"ping -c {s.validated_data['count']} example.com", shell=True)
    subprocess.run([s.validated_data["target"], "-h"])  # caught


def run_form_tool(request):
    form = ToolForm(request.POST)
    if form.is_valid():
        subprocess.run([form.cleaned_data["tool"]])
        subprocess.run(f"ls {form.cleaned_data['target']}", shell=True)  # caught
