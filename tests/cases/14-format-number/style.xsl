<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output method="text" encoding="UTF-8"/>
<xsl:decimal-format name="eu" decimal-separator="," grouping-separator="."/>
<xsl:decimal-format name="custom" digit="x" minus-sign="~" NaN="nothing" infinity="forever" zero-digit="0"/>
<xsl:template match="/">
<xsl:text>[#,##0.00] [1234567.891] = </xsl:text><xsl:value-of select="format-number(1234567.891, &quot;#,##0.00&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0%] [0.5] = </xsl:text><xsl:value-of select="format-number(0.5, &quot;0%&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[000;(000)] [-3] = </xsl:text><xsl:value-of select="format-number(-3, &quot;000;(000)&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [0] = </xsl:text><xsl:value-of select="format-number(0, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[.##] [0.256] = </xsl:text><xsl:value-of select="format-number(0.256, &quot;.##&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#.#] [12] = </xsl:text><xsl:value-of select="format-number(12, &quot;#.#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.0] [-0.04] = </xsl:text><xsl:value-of select="format-number(-0.04, &quot;0.0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>['#'0] [5] = </xsl:text><xsl:value-of select="format-number(5, &quot;'#'0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [0 div 0] = </xsl:text><xsl:value-of select="format-number(0 div 0, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [-1 div 0] = </xsl:text><xsl:value-of select="format-number(-1 div 0, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#0#] [1.5] = </xsl:text><xsl:value-of select="format-number(1.5, &quot;#0#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#,##0.###] [-1234.5678] = </xsl:text><xsl:value-of select="format-number(-1234.5678, &quot;#,##0.###&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.000] [3.14159] = </xsl:text><xsl:value-of select="format-number(3.14159, &quot;0.000&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[00.00] [1.5] = </xsl:text><xsl:value-of select="format-number(1.5, &quot;00.00&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#.00] [0.5] = </xsl:text><xsl:value-of select="format-number(0.5, &quot;#.00&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [0.5] = </xsl:text><xsl:value-of select="format-number(0.5, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [1.5] = </xsl:text><xsl:value-of select="format-number(1.5, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [2.5] = </xsl:text><xsl:value-of select="format-number(2.5, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [-2.5] = </xsl:text><xsl:value-of select="format-number(-2.5, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0] [-0] = </xsl:text><xsl:value-of select="format-number(-0, &quot;0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#,###] [1234567] = </xsl:text><xsl:value-of select="format-number(1234567, &quot;#,###&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#,##,###] [1234567] = </xsl:text><xsl:value-of select="format-number(1234567, &quot;#,##,###&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.###E0] [12345] = </xsl:text><xsl:value-of select="format-number(12345, &quot;0.###E0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#%] [0.256] = </xsl:text><xsl:value-of select="format-number(0.256, &quot;#%&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#‰] [0.256] = </xsl:text><xsl:value-of select="format-number(0.256, &quot;#‰&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#.##%] [0.12345] = </xsl:text><xsl:value-of select="format-number(0.12345, &quot;#.##%&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[¤#,##0.00] [12.5] = </xsl:text><xsl:value-of select="format-number(12.5, &quot;¤#,##0.00&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#;-#] [-5] = </xsl:text><xsl:value-of select="format-number(-5, &quot;#;-#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#;#] [-5] = </xsl:text><xsl:value-of select="format-number(-5, &quot;#;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#;(#)] [5] = </xsl:text><xsl:value-of select="format-number(5, &quot;#;(#)&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.0;[0.0]] [-2.25] = </xsl:text><xsl:value-of select="format-number(-2.25, &quot;0.0;[0.0]&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>['%'#] [5] = </xsl:text><xsl:value-of select="format-number(5, &quot;'%'#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#'%'] [5] = </xsl:text><xsl:value-of select="format-number(5, &quot;#'%'&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#.0#] [1] = </xsl:text><xsl:value-of select="format-number(1, &quot;#.0#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#.0#] [1.234] = </xsl:text><xsl:value-of select="format-number(1.234, &quot;#.0#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[.0] [0] = </xsl:text><xsl:value-of select="format-number(0, &quot;.0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[##0.0#] [0.004] = </xsl:text><xsl:value-of select="format-number(0.004, &quot;##0.0#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.00] [123456789012.345] = </xsl:text><xsl:value-of select="format-number(123456789012.345, &quot;0.00&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#] [1e20] = </xsl:text><xsl:value-of select="format-number(1e20, &quot;#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.0] [1e-10] = </xsl:text><xsl:value-of select="format-number(1e-10, &quot;0.0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[] [5] = </xsl:text><xsl:value-of select="format-number(5, &quot;&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[abc] [5] = </xsl:text><xsl:value-of select="format-number(5, &quot;abc&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#.#.#] [1] = </xsl:text><xsl:value-of select="format-number(1, &quot;#.#.#&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0] [99999999999999999] = </xsl:text><xsl:value-of select="format-number(99999999999999999, &quot;0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[#,##0] [-0.4] = </xsl:text><xsl:value-of select="format-number(-0.4, &quot;#,##0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.0] [9.95] = </xsl:text><xsl:value-of select="format-number(9.95, &quot;0.0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.0] [0.05] = </xsl:text><xsl:value-of select="format-number(0.05, &quot;0.0&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>[0.00] [1.005] = </xsl:text><xsl:value-of select="format-number(1.005, &quot;0.00&quot;)"/><xsl:text>&#10;</xsl:text>
<xsl:text>eu [#.##0,00] [1234567.891] = </xsl:text><xsl:value-of select="format-number(1234567.891, &quot;#.##0,00&quot;, 'eu')"/><xsl:text>&#10;</xsl:text>
<xsl:text>eu [#.##0,###;-#.##0,###] [-1234.5] = </xsl:text><xsl:value-of select="format-number(-1234.5, &quot;#.##0,###;-#.##0,###&quot;, 'eu')"/><xsl:text>&#10;</xsl:text>
<xsl:text>custom [x##0] [42] = </xsl:text><xsl:value-of select="format-number(42, &quot;x##0&quot;, 'custom')"/><xsl:text>&#10;</xsl:text>
<xsl:text>custom [x##0;neg x##0] [-42] = </xsl:text><xsl:value-of select="format-number(-42, &quot;x##0;neg x##0&quot;, 'custom')"/><xsl:text>&#10;</xsl:text>
<xsl:text>custom NaN = </xsl:text><xsl:value-of select="format-number(0 div 0, '#', 'custom')"/><xsl:text>&#10;</xsl:text>
<xsl:text>custom -Inf = </xsl:text><xsl:value-of select="format-number(-1 div 0, '#', 'custom')"/><xsl:text>&#10;</xsl:text>
</xsl:template></xsl:stylesheet>
