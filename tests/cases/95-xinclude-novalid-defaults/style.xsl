<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><r main="{doc/@a}" included="{//literal/@moreinfo}"/></xsl:template>
</xsl:stylesheet>
