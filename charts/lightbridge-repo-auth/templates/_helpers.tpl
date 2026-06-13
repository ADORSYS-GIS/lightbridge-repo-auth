{{- define "lra.name" -}}lightbridge-repo-auth{{- end -}}

{{- define "lra.labels" -}}
app.kubernetes.io/name: {{ include "lra.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "lra.selectorLabels" -}}
app.kubernetes.io/name: {{ include "lra.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "lra.image" -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{ .Values.image.repository }}:{{ $tag }}
{{- end -}}
