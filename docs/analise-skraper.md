# Análise funcional do Skraper 1.4.2

> Registro histórico da versão 0.1. A versão 0.2 substituiu o ScreenScraper por SteamGridDB e acrescentou gamelist.xml. Consulte o README para o comportamento atual.

Inspeção realizada em 18/10/2026, sobre uma instalação local do Skraper 1.4.2. Foram lidos os arquivos de release e um descritor XML de composição; não foi executado o programa nem extraído código ou credenciais do executável. Portanto, esta é uma reprodução independente do fluxo documentado e observado nos artefatos, não uma tradução do código interno.

## Evidências locais

- `SkraperUI.exe`, `SkraperLibrary2.dll`, dependências .NET e `SharpCompress.dll` fazem parte da distribuição. As notas mencionam suporte Mono e inspeção de arquivos compactados.
- `ReleaseNote.txt`, seção 1.4.2: correções de redimensionamento, nomes de arquivos em requisições, cálculo opcional de CRC, cache de hashes e conversão JSON.
- Seção 1.4: filtro de `.cue`, `.m3u`, `.ccd`, `.sub` dentro de arquivos compactados.
- Seções anteriores: prioridades regionais, cache de requisições e mídias, limites da API, gamelists, imagens compostas, vídeos e manuais.
- `Mixes/3 Images Mix [HD][4-3].xml`: viewport 1280×960, screenshot com alternativas, wheel no canto inferior direito e caixa 3D no inferior esquerdo. O XML tem ancoragem, transparência e recursos de fallback. O mix Rust reproduz a ideia de composição, com geometria própria; não interpreta esse XML.

## Fluxo reproduzido

Selecionar sistema/pasta → listar ROMs → calcular identificação → consultar catálogo → escolher tipo/região → baixar → redimensionar ou compor → salvar PNG → mostrar progresso e prévia.

O catálogo e as imagens vêm do ScreenScraper. A consulta de jogo da API recebe credenciais próprias do aplicativo, sistema, nome, tamanho e hashes e retorna metadados/mídias. Usamos os campos JSON `response.jeu`, `noms` e `medias`; erros de autenticação/cota interrompem o lote. A aceitação pelo serviço deve ser confirmada com credenciais válidas.

## Comparação de escopo

| Função | Projeto Rust |
|---|---|
| Seleção de pasta e sistema | Interface gráfica, uma pasta/sistema por lote |
| Identificação por arquivo | CRC32 + MD5 + SHA-1 |
| Compactados | ZIP simples; múltiplos candidatos requerem revisão |
| Escolha de mídia/região | Implementada com ordem configurável |
| Capas, logos e screenshots | Implementados |
| Composições | Mixes do servidor e mix local de três imagens |
| Redimensionamento | PNG, proporção preservada, sem recorte |
| Cache | Bytes de imagens, não metadados/hashes |
| Limites de serviço | Uma requisição por vez; parada ao receber limitação |
| Interface | Português, estilo Breeze claro, worker em segundo plano |
| Gamelists e demais frontends | Não implementados |
| Motor XML, vídeos e manuais | Não implementados |

## Fontes oficiais

- [Skraper: apresentação e recursos](https://skraper.net/index.html?lng=en).
- [ScreenScraper: documentação da API v2](https://www.screenscraper.fr/webapi2.php).

Não foram modificadas ROMs, configurações ou arquivos da instalação original.
